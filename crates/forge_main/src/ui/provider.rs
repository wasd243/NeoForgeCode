use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use convert_case::{Case, Casing};
use forge_api::{
    API, AnyProvider, ApiKeyRequest, AuthContextRequest, AuthContextResponse, CodeRequest,
    ConfigOperation, DeviceCodeRequest, ModelId, Provider, ProviderId,
};
use forge_config::ForgeConfig;
use forge_domain::{AuthMethod, ConsoleWriter, TitleFormat};
use forge_select::{ForgeWidget, SelectRow};
use url::Url;

use super::UI;
use crate::display_constants::{markers, status};
use crate::error::UIError;
use crate::info::Info;
use crate::porcelain::Porcelain;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    pub(super) async fn handle_provider_command(
        &mut self,
        provider_group: crate::cli::ProviderCommandGroup,
    ) -> anyhow::Result<()> {
        use crate::cli::ProviderCommand;

        match provider_group.command {
            ProviderCommand::Login { provider } => {
                self.handle_provider_login(provider.as_ref()).await?;
            }
            ProviderCommand::Logout { provider } => {
                self.handle_provider_logout(provider.as_ref()).await?;
            }
            ProviderCommand::List { types } => {
                self.on_show_providers(provider_group.porcelain, types)
                    .await?;
            }
        }

        Ok(())
    }

    pub(super) async fn handle_provider_login(
        &mut self,
        provider_id: Option<&ProviderId>,
    ) -> anyhow::Result<()> {
        // Get the provider to login to
        let any_provider = if let Some(id) = provider_id {
            // Specific provider requested
            self.api.get_provider(id).await?
        } else {
            // Fetch all providers for selection (no type filter, like shell :login)
            let providers = self.api.get_providers().await?;

            match self.select_provider_from_list(providers, "Provider", None, None)? {
                Some(provider) => provider,
                None => {
                    self.writeln_title(TitleFormat::info("Cancelled"))?;
                    return Ok(());
                }
            }
        };

        // For login, always configure (even if already configured) to allow
        // re-authentication
        let provider = match self
            .configure_provider(any_provider.id(), any_provider.auth_methods().to_vec())
            .await?
        {
            Some(provider) => provider,
            None => return Ok(()),
        };

        // Set as default and handle model selection
        self.finalize_provider_activation(provider, None).await
    }

    pub(super) async fn handle_provider_logout(
        &mut self,
        provider_id: Option<&ProviderId>,
    ) -> anyhow::Result<bool> {
        // If provider_id is specified, logout from that specific provider
        if let Some(id) = provider_id {
            let provider = self.api.get_provider(id).await?;

            if !provider.is_configured() {
                return Err(anyhow::anyhow!("Provider '{id}' is not configured"));
            }
            self.api.remove_provider(id).await?;
            self.writeln_title(TitleFormat::debug(format!(
                "Successfully logged out from {id}"
            )))?;
            return Ok(true);
        }

        // Fetch and filter configured providers (like shell :logout filters to status
        // [yes])
        let configured_providers: Vec<AnyProvider> = self
            .api
            .get_providers()
            .await?
            .into_iter()
            .filter(|p| p.is_configured())
            .collect();

        if configured_providers.is_empty() {
            self.writeln_title(TitleFormat::info("No configured providers found"))?;
            return Ok(false);
        }

        match self.select_provider_from_list(configured_providers, "Provider", None, None)? {
            Some(provider) => {
                let provider_id = provider.id();
                self.api.remove_provider(&provider_id).await?;
                self.writeln_title(TitleFormat::debug(format!(
                    "Successfully logged out from {provider_id}"
                )))?;
                return Ok(true);
            }
            None => {
                self.writeln_title(TitleFormat::info("Cancelled"))?;
            }
        }

        Ok(false)
    }

    pub(super) async fn handle_api_key_input(
        &mut self,
        provider_id: ProviderId,
        request: &ApiKeyRequest,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        self.spinner.stop(None)?;

        // Extract existing API key and URL params for prefilling
        let existing_url_params = request.existing_params.as_ref();

        // Collect URL parameters if required
        let url_params = request
            .required_params
            .iter()
            .map(|param| {
                let param_value = if let Some(options) = &param.options {
                    // Dropdown path: user selects from preset options
                    let starting = existing_url_params
                        .and_then(|p| p.get(&param.name))
                        .and_then(|v| options.iter().position(|o| o.as_str() == v.as_str()))
                        .unwrap_or(0);
                    ForgeWidget::select(format!("Select {}", param.name), options.clone())
                        .with_starting_cursor(starting)
                        .prompt()?
                        .context("Parameter selection cancelled")?
                } else {
                    // Free-text path
                    let label = if param.optional {
                        format!("Enter {} (optional, press Enter to skip)", param.name)
                    } else {
                        format!("Enter {}", param.name)
                    };
                    let mut input = ForgeWidget::input(label);

                    // Add default value if it exists in the credential
                    if let Some(params) = existing_url_params
                        && let Some(default_value) = params.get(&param.name)
                    {
                        input = input.with_default(default_value.as_str());
                    }

                    if param.optional {
                        input = input.allow_empty(true);
                    }

                    let param_value = input.prompt()?.context("Parameter input cancelled")?;

                    if !param.optional {
                        anyhow::ensure!(
                            !param_value.trim().is_empty(),
                            "{} cannot be empty",
                            param.name
                        );
                    }

                    param_value.trim_end_matches('/').to_string()
                };

                Ok((param.name.to_string(), param_value))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?;

        let allows_local_api_key = matches!(
            provider_id.as_ref().as_ref(),
            "ollama" | "vllm" | "lm_studio" | "llama_cpp" | "jan_ai"
        );

        // Check if API key is already provided
        // For Google ADC, we use a marker to skip prompting
        // For other providers, we use the existing key as a default value (autofill)
        let api_key_str = if let Some(default_key) = &request.api_key {
            let key_str = default_key.as_ref();

            // Skip prompting for markers that indicate non-API-key auth
            if key_str == "google_adc_marker" || key_str == "aws_profile_marker" {
                key_str.to_string()
            } else if allows_local_api_key {
                let input = ForgeWidget::input(format!(
                    "Enter your {provider_id} API key (press Enter to skip)"
                ))
                .allow_empty(true);
                let api_key = input.prompt()?.context("API key input cancelled")?;
                let api_key_str = api_key.trim();

                if api_key_str.is_empty() {
                    "local".to_string()
                } else {
                    api_key_str.to_string()
                }
            } else {
                // For other providers, show the existing key as default (autofill)
                let input = ForgeWidget::input(format!("Enter your {provider_id} API key"))
                    .with_default(key_str);
                let api_key = input.prompt()?.context("API key input cancelled")?;
                let api_key_str = api_key.trim();
                anyhow::ensure!(!api_key_str.is_empty(), "API key cannot be empty");
                api_key_str.to_string()
            }
        } else if allows_local_api_key {
            let input = ForgeWidget::input(format!(
                "Enter your {provider_id} API key (press Enter to skip)"
            ))
            .allow_empty(true);
            let api_key = input.prompt()?.context("API key input cancelled")?;
            let api_key_str = api_key.trim();

            if api_key_str.is_empty() {
                "local".to_string()
            } else {
                api_key_str.to_string()
            }
        } else {
            // Prompt for API key input (no existing key)
            let input = ForgeWidget::input(format!("Enter your {provider_id} API key"));
            let api_key = input.prompt()?.context("API key input cancelled")?;
            let api_key_str = api_key.trim();
            anyhow::ensure!(!api_key_str.is_empty(), "API key cannot be empty");
            api_key_str.to_string()
        };

        // Update the context with collected data
        let response = AuthContextResponse::api_key(request.clone(), &api_key_str, url_params);

        self.api
            .complete_provider_auth(
                provider_id,
                response,
                Duration::from_secs(0), // No timeout needed since we have the data
            )
            .await?;

        Ok(())
    }

    pub(super) fn display_oauth_device_info_new(
        &mut self,
        user_code: &str,
        verification_uri: &str,
        verification_uri_complete: Option<&str>,
    ) -> anyhow::Result<()> {
        use colored::Colorize;

        let display_uri = verification_uri_complete.unwrap_or(verification_uri);

        self.writeln("")?;
        self.writeln(format!(
            "{} Please visit: {}",
            "→".blue(),
            display_uri.blue().underline()
        ))?;
        // Try to copy code to clipboard automatically (not available on Android)
        #[cfg(not(target_os = "android"))]
        let clipboard_copied = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(user_code))
            .is_ok();

        #[cfg(target_os = "android")]
        let clipboard_copied = false;

        if clipboard_copied {
            self.writeln(format!(
                "{} Code copied to clipboard: {}",
                "✓".green().bold(),
                user_code.bold().yellow()
            ))?;
        } else {
            self.writeln(format!(
                "{} Enter code: {}",
                "→".blue(),
                user_code.bold().yellow()
            ))?;
        }
        self.writeln("")?;

        // Try to open browser automatically
        if let Err(e) = open::that(display_uri) {
            self.writeln_title(TitleFormat::error(format!(
                "Failed to open browser automatically: {e}"
            )))?;
        }

        Ok(())
    }

    pub(super) async fn handle_device_flow(
        &mut self,
        provider_id: ProviderId,
        request: &DeviceCodeRequest,
    ) -> Result<()> {
        use std::time::Duration;

        let user_code = request.user_code.clone();
        let verification_uri = request.verification_uri.clone();
        let verification_uri_complete = request.verification_uri_complete.clone();

        self.spinner.stop(None)?;
        // Display OAuth device information
        self.display_oauth_device_info_new(
            user_code.as_ref(),
            verification_uri.as_ref(),
            verification_uri_complete.as_ref().map(|v| v.as_ref()),
        )?;

        // Step 2: Complete authentication (polls if needed for OAuth flows)
        self.spinner.start(Some("Completing authentication..."))?;

        let response = AuthContextResponse::device_code(request.clone());

        self.api
            .complete_provider_auth(provider_id, response, Duration::from_secs(600))
            .await?;

        self.spinner.stop(None)?;

        Ok(())
    }

    pub(super) async fn display_credential_success(
        &mut self,
        provider_id: ProviderId,
    ) -> anyhow::Result<()> {
        self.writeln_title(TitleFormat::info(format!(
            "{provider_id} configured successfully"
        )))?;

        Ok(())
    }

    pub(super) async fn handle_code_flow(
        &mut self,
        provider_id: ProviderId,
        request: &CodeRequest,
    ) -> anyhow::Result<()> {
        use colored::Colorize;

        self.spinner.stop(None)?;

        self.writeln(format!(
            "{}",
            format!("Authenticate using your {provider_id} account").dimmed()
        ))?;

        let callback_server =
            match crate::oauth_callback::LocalhostOAuthCallbackServer::start(request) {
                Ok(Some(server)) => {
                    self.writeln(format!(
                        "{} Waiting for browser callback on {}",
                        "→".blue(),
                        server.redirect_uri().as_str().blue().underline()
                    ))?;
                    Some(server)
                }
                Ok(None) | Err(_) => {
                    // Not a localhost callback flow, or the listener could not be
                    // started — fall back to manual code paste.
                    None
                }
            };

        // Display authorization URL
        self.writeln(format!(
            "{} Please visit: {}",
            "→".blue(),
            request.authorization_url.as_str().blue().underline()
        ))?;

        // Try to open browser automatically
        if let Err(e) = open::that(request.authorization_url.as_str()) {
            self.writeln_title(TitleFormat::error(format!(
                "Failed to open browser automatically: {e}"
            )))?;
        }

        let code = if let Some(server) = callback_server {
            server.wait_for_code().await?
        } else {
            // Prompt user to paste authorization code
            let code =
                ForgeWidget::input(format!("Paste the authorization code for {provider_id}"))
                    .prompt()?
                    .ok_or_else(|| anyhow::anyhow!("Authorization code input cancelled"))?;

            if code.trim().is_empty() {
                anyhow::bail!("Authorization code cannot be empty");
            }

            code
        };

        self.spinner
            .start(Some("Exchanging authorization code..."))?;

        let response = AuthContextResponse::code(request.clone(), &code);

        self.api
            .complete_provider_auth(
                provider_id,
                response,
                Duration::from_secs(0), // No timeout needed since we have the data
            )
            .await?;

        self.spinner.stop(None)?;

        Ok(())
    }

    /// Helper method to select an authentication method when multiple are
    /// available
    pub(super) async fn select_auth_method(
        &mut self,
        provider_id: ProviderId,
        auth_methods: &[AuthMethod],
    ) -> Result<Option<AuthMethod>> {
        use colored::Colorize;

        if auth_methods.is_empty() {
            return Err(UIError::NoAuthMethodsAvailable { provider: provider_id.clone() }.into());
        }

        // If only one auth method, use it directly
        if auth_methods.len() == 1 {
            let Some(method) = auth_methods.first() else {
                return Err(
                    UIError::NoAuthMethodsAvailable { provider: provider_id.clone() }.into(),
                );
            };
            return Ok(Some(method.clone()));
        }

        // Multiple auth methods - ask user to choose
        self.spinner.stop(None)?;

        self.writeln_title(TitleFormat::action(format!("Configure {provider_id}")))?;
        self.writeln("Multiple authentication methods available".dimmed())?;

        let method_names: Vec<String> = auth_methods
            .iter()
            .map(|method| match method {
                AuthMethod::ApiKey => "API Key".to_string(),
                AuthMethod::OAuthDevice(_) => "OAuth Device Flow".to_string(),
                AuthMethod::OAuthCode(_) => "OAuth Authorization Code".to_string(),
                AuthMethod::GoogleAdc => "Google Application Default Credentials (ADC)".to_string(),
                AuthMethod::AwsProfile => "AWS Profile (SSO/IAM)".to_string(),
                AuthMethod::CodexDevice(_) => "OpenAI Codex Device Flow".to_string(),
            })
            .collect();

        match ForgeWidget::select("Select authentication method:", method_names.clone())
            .with_help_message("Use arrow keys to navigate and Enter to select")
            .prompt()?
        {
            Some(selected_name) => {
                // Find the corresponding auth method
                let Some(index) = method_names.iter().position(|name| name == &selected_name)
                else {
                    return Err(UIError::AuthMethodNotFound.into());
                };
                let Some(method) = auth_methods.get(index) else {
                    return Err(UIError::AuthMethodNotFound.into());
                };
                Ok(Some(method.clone()))
            }
            None => Ok(None),
        }
    }

    /// Creates ForgeCode Services credentials if not already authenticated and
    /// displays the credentials file location to the user.
    pub(super) async fn init_forge_services(&mut self) -> Result<()> {
        self.api.create_auth_credentials().await?;
        let env = self.api.environment();
        let credentials_path = crate::info::format_path_for_display(&env, &env.credentials_path());
        self.writeln_title(
            TitleFormat::info("ForgeCode Services enabled").sub_title(&credentials_path),
        )?;
        Ok(())
    }

    /// Handle authentication flow for an unavailable provider
    pub(super) async fn configure_provider(
        &mut self,
        provider_id: ProviderId,
        auth_methods: Vec<AuthMethod>,
    ) -> Result<Option<Provider<Url>>> {
        if provider_id == ProviderId::FORGE_SERVICES {
            self.init_forge_services().await?;
            return Ok(None);
        }
        // Select auth method (or use the only one available)
        let auth_method = match self
            .select_auth_method(provider_id.clone(), &auth_methods)
            .await?
        {
            Some(method) => method,
            None => return Ok(None), // User cancelled
        };

        // Show warning for Claude Code provider about account ban risk
        if provider_id == ProviderId::CLAUDE_CODE {
            self.writeln_title(
                TitleFormat::warning(
                    "Using Claude Code subscription in third-party tools violates Anthropic's Terms of Service."
                )
                .sub_title("Your account may be suspended or banned. Continue at your own risk."),
            )?;

            let confirmed = ForgeWidget::confirm("Do you want to continue with this provider?")
                .with_default(false)
                .prompt()?;

            if !confirmed.unwrap_or(false) {
                return Ok(None);
            }
        }

        self.spinner.start(Some("Initiating authentication..."))?;
        // Initiate the authentication flow
        let auth_request = self
            .api
            .init_provider_auth(provider_id.clone(), auth_method)
            .await?;

        // Handle the specific authentication flow based on the request type
        match auth_request {
            AuthContextRequest::ApiKey(request) => {
                self.handle_api_key_input(provider_id.clone(), &request)
                    .await?;
            }
            AuthContextRequest::DeviceCode(request) => {
                self.handle_device_flow(provider_id.clone(), &request)
                    .await?;
            }
            AuthContextRequest::Code(request) => {
                self.handle_code_flow(provider_id.clone(), &request).await?;
            }
        }

        // Verify by fetching the configured provider
        let provider = self.api.get_provider(&provider_id).await?;

        self.display_credential_success(provider_id.clone()).await?;

        Ok(provider.into_configured())
    }

    /// Builds a porcelain-style provider selection list from a set of
    /// providers, displays it in the interactive picker, and returns the
    /// selected provider.
    ///
    /// The display matches the shell plugin's `_forge_select_provider`:
    /// columns NAME, HOST, TYPE, LOGGED IN (hiding the raw ID column).
    pub(super) fn select_provider_from_list(
        &self,
        providers: Vec<AnyProvider>,
        prompt: &str,
        current_provider_id: Option<ProviderId>,
        query: Option<String>,
    ) -> Result<Option<AnyProvider>> {
        if providers.is_empty() {
            return Ok(None);
        }

        // Sort providers alphabetically by display name
        let mut sorted = providers;
        sorted.sort_by_key(|a| a.id().to_string());

        // Build Info structure (same as on_show_providers)
        let mut info = Info::new();
        for provider in &sorted {
            let id: &str = &provider.id();
            let display_name = provider.id().to_string();
            let domain = if let Some(url) = provider.url() {
                url.domain().map(|d| d.to_string()).unwrap_or_default()
            } else {
                markers::EMPTY.to_string()
            };
            let provider_type = provider.provider_type().to_string();
            let configured = provider.is_configured();
            info = info
                .add_title(id.to_case(Case::UpperSnake))
                .add_key_value("name", display_name)
                .add_key_value("id", id)
                .add_key_value("host", domain)
                .add_key_value("type", provider_type);
            if configured {
                info = info.add_key_value("logged in", status::YES);
            }
        }

        // Convert to porcelain, drop title (col 0) and raw id (col 2)
        let porcelain_output = Porcelain::from(&info)
            .drop_cols(&[0, 2])
            .uppercase_headers();
        let porcelain_str = porcelain_output.to_string();

        let all_lines: Vec<&str> = porcelain_str.lines().collect();
        if all_lines.is_empty() {
            return Ok(None);
        }

        let Some(header) = all_lines.first() else {
            return Err(UIError::MissingHeaderLine.into());
        };
        let mut rows = vec![SelectRow::header(header.to_string())];
        for (index, line) in all_lines.iter().skip(1).enumerate() {
            if let Some(provider) = sorted.get(index) {
                rows.push(SelectRow::new(
                    provider.id().as_ref().to_string(),
                    line.to_string(),
                ));
            }
        }

        let selected = self.select_raw_row(
            prompt,
            query,
            rows,
            1,
            current_provider_id.map(|current| current.as_ref().to_string()),
        )?;

        Ok(selected.and_then(|row| {
            sorted
                .into_iter()
                .find(|provider| provider.id().as_ref().as_ref() == row.raw)
        }))
    }

    /// Selects a provider, optionally configuring it if not already configured.
    pub(super) async fn select_provider(
        &mut self,
        query: Option<String>,
        configured_only: bool,
    ) -> Result<Option<AnyProvider>> {
        let mut providers: Vec<AnyProvider> = self
            .api
            .get_providers()
            .await?
            .into_iter()
            .filter(|p| {
                let filter = forge_domain::ProviderType::Llm;
                match &p {
                    AnyProvider::Url(provider) => provider.provider_type == filter,
                    AnyProvider::Template(provider) => provider.provider_type == filter,
                }
            })
            .collect();

        if configured_only {
            providers.retain(|provider| provider.is_configured());
        }

        if providers.is_empty() {
            return Err(anyhow::anyhow!("No AI provider API keys configured"));
        }

        let current_provider_id = self
            .get_provider(self.api.get_active_agent().await)
            .await
            .ok()
            .map(|p| p.id);

        self.select_provider_from_list(providers, "Provider", current_provider_id, query)
    }

    pub(super) async fn on_provider_selection(&mut self) -> Result<bool> {
        // Select a provider
        // If no provider was selected (user canceled), return early
        let any_provider = match self.select_provider(None, false).await? {
            Some(provider) => provider,
            None => return Ok(false),
        };

        self.activate_provider(any_provider).await?;
        // Check if provider was actually saved — if user cancelled model selection
        // inside activate_provider, nothing was written
        Ok(self.api.get_session_config().await.is_some())
    }

    /// Activates a provider by configuring it if needed, setting it as default,
    /// and ensuring a compatible model is selected.
    pub(super) async fn activate_provider(&mut self, any_provider: AnyProvider) -> Result<()> {
        self.activate_provider_with_model(any_provider, None).await
    }

    /// Activates a provider with an optional pre-selected model.
    /// When `model` is provided, the interactive model selection prompt is
    /// skipped and the specified model is set directly.
    pub(super) async fn activate_provider_with_model(
        &mut self,
        any_provider: AnyProvider,
        model: Option<ModelId>,
    ) -> Result<()> {
        // Trigger authentication for the selected provider only if not configured
        let provider = if !any_provider.is_configured() {
            match self
                .configure_provider(any_provider.id(), any_provider.auth_methods().to_vec())
                .await?
            {
                Some(provider) => provider,
                None => return Ok(()),
            }
        } else {
            // Provider is already configured, convert it
            match any_provider.into_configured() {
                Some(provider) => provider,
                None => return Ok(()),
            }
        };

        // Set as default and handle model selection
        self.finalize_provider_activation(provider, model).await
    }

    /// Finalizes provider activation by setting it as default and ensuring
    /// a compatible model is selected.
    /// When `model` is `Some`, the interactive model selection is skipped and
    /// the provided model is validated and set directly.
    pub(super) async fn finalize_provider_activation(
        &mut self,
        provider: Provider<Url>,
        model: Option<ModelId>,
    ) -> Result<()> {
        // If a model was pre-selected (e.g. from :model), validate and set it
        // directly without prompting
        if let Some(model) = model {
            let model_id = self
                .validate_model(model.as_str(), Some(&provider.id))
                .await?;
            self.api
                .update_config(vec![ConfigOperation::SetSessionConfig(
                    forge_domain::ModelConfig::new(provider.id.clone(), model_id.clone()),
                )])
                .await?;
            self.writeln_title(
                TitleFormat::action(format!("{}", provider.id))
                    .sub_title("is now the default provider"),
            )?;
            self.writeln_title(
                TitleFormat::action(model_id.as_str()).sub_title("is now the default model"),
            )?;
            return Ok(());
        }

        // Check if the current model is available for the new provider
        let current_model = self.api.get_session_config().await.map(|c| c.model);
        let (needs_model_selection, compatible_model) = match current_model {
            None => (true, None),
            Some(current_model) => {
                let provider_models = self.api.get_all_provider_models().await?;
                let model_available = provider_models
                    .iter()
                    .find(|pm| pm.provider_id == provider.id)
                    .map(|pm| pm.models.iter().any(|m| m.id == current_model))
                    .unwrap_or(false);
                if model_available {
                    (false, Some(current_model))
                } else {
                    (true, None)
                }
            }
        };

        if needs_model_selection {
            let selected = self.on_model_selection(Some(provider.id.clone())).await?;
            if selected.is_none() {
                // User cancelled — preserve existing config untouched
                return Ok(());
            }
        } else {
            // The current model is compatible with the new provider — write both
            // atomically so the session always stores a consistent pair.
            let model =
                compatible_model.expect("compatible_model is Some when !needs_model_selection");
            self.api
                .update_config(vec![ConfigOperation::SetSessionConfig(
                    forge_domain::ModelConfig::new(provider.id.clone(), model),
                )])
                .await?;

            self.writeln_title(
                TitleFormat::action(format!("{}", provider.id))
                    .sub_title("is now the default provider"),
            )?;
        }

        Ok(())
    }
}

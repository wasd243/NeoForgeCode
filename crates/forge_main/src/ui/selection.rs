use anyhow::{Context, Result};
use forge_api::{API, AgentId, ConfigOperation, ModelId, ProviderId};
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, TitleFormat};
use forge_select::SelectRow;

use super::UI;
use crate::display_constants::{markers, status};
use crate::error::UIError;
use crate::info::Info;
use crate::porcelain::Porcelain;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    pub(super) async fn select_agent(&self, query: Option<String>) -> Result<Option<AgentId>> {
        let rows = self.agent_select_rows().await?;
        let initial_raw = self
            .api
            .get_active_agent()
            .await
            .map(|current| current.as_str().to_string());

        Ok(self
            .select_raw_row("Agent", query, rows, 1, initial_raw)?
            .map(|row| AgentId::new(row.raw)))
    }

    pub(super) async fn agent_select_rows(&self) -> Result<Vec<SelectRow>> {
        let info = self.build_agents_info(false).await?;
        let porcelain = Porcelain::from(&info)
            .drop_cols(&[0, 3])
            .truncate(3, 30)
            .uppercase_headers();

        Self::porcelain_rows(porcelain)
    }

    pub(super) async fn select_reasoning_effort(
        &self,
        prompt: &str,
        query: Option<String>,
    ) -> anyhow::Result<Option<String>> {
        let effort_levels = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
        let current_effort = self.api.get_reasoning_effort().await.ok().flatten();
        let current_str = current_effort.as_ref().map(|e| e.to_string());
        let rows = effort_levels
            .iter()
            .map(|level| SelectRow::new(*level, *level))
            .collect();

        Ok(self
            .select_raw_row(prompt, query, rows, 0, current_str)?
            .map(|row| row.raw))
    }

    /// Select a model from all configured providers using porcelain-style
    /// tabular display matching the shell plugin's `:model` UI.
    ///
    /// Shows columns: MODEL, PROVIDER, CONTEXT WINDOW, TOOL SUPPORTED, IMAGE
    /// with a non-selectable header row.
    ///
    /// When `provider_filter` is `Some`, only models belonging to that provider
    /// are shown. This is used during onboarding so that after a provider is
    /// selected the model list is scoped to that provider only.
    ///
    /// # Returns
    /// - `Ok(Some((ModelId, ProviderId)))` if a model was selected, carrying
    ///   both the model and the provider it belongs to
    /// - `Ok(None)` if selection was canceled
    #[async_recursion::async_recursion]
    pub(super) async fn select_model(
        &mut self,
        provider_filter: Option<ProviderId>,
        query: Option<String>,
    ) -> Result<Option<(ModelId, ProviderId)>> {
        // Check if provider is set otherwise first ask to select a provider
        if provider_filter.is_none() && self.api.get_session_config().await.is_none() {
            if !self.on_provider_selection().await? {
                return Ok(None);
            }

            // Provider activation may have already completed model selection.
            // If it did not, continue below and show the full cross-provider
            // model list.
            if self.api.get_session_config().await.is_some() {
                return Ok(None);
            }
        }

        // Fetch models from ALL configured providers (matches shell plugin's
        // `forge list models --porcelain`), then optionally filter by provider.
        self.spinner.start(Some("Loading"))?;
        let mut all_provider_models = self.api.get_all_provider_models().await?;
        self.spinner.stop(None)?;

        // When a provider filter is specified (e.g. during onboarding after a
        // provider was just selected), restrict the list to that provider's
        // models so the user cannot accidentally pick a model from a different
        // provider.
        if let Some(ref filter_id) = provider_filter {
            all_provider_models.retain(|pm| &pm.provider_id == filter_id);
        }

        if all_provider_models.is_empty() {
            return Ok(None);
        }

        // Sort models and providers (same as on_show_models)
        all_provider_models
            .iter_mut()
            .for_each(|pm| pm.models.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str())));
        all_provider_models.sort_by(|a, b| a.provider_id.as_ref().cmp(b.provider_id.as_ref()));

        // Build the same Info structure as on_show_models, then convert to
        // Porcelain for tabular display.
        let mut info = Info::new();
        for pm in &all_provider_models {
            let provider_display = pm.provider_id.to_string();
            for model in &pm.models {
                let id = model.id.to_string();
                info = info
                    .add_title(&id)
                    .add_key_value("Model", model.name.as_ref().unwrap_or(&id))
                    .add_key_value("Provider", &provider_display);

                if let Some(limit) = model.context_length {
                    let context = if limit >= 1_000_000 {
                        format!("{}M", limit / 1_000_000)
                    } else if limit >= 1000 {
                        format!("{}k", limit / 1000)
                    } else {
                        format!("{limit}")
                    };
                    info = info.add_key_value("Context Window", context);
                } else {
                    info = info.add_key_value("Context Window", markers::EMPTY);
                }

                if let Some(supported) = model.tools_supported {
                    info = info.add_key_value(
                        "Tool Supported",
                        if supported { status::YES } else { status::NO },
                    );
                } else {
                    info = info.add_key_value("Tools", markers::EMPTY);
                }

                let supports_image = model
                    .input_modalities
                    .contains(&forge_domain::InputModality::Image);
                info = info.add_key_value(
                    "Image",
                    if supports_image {
                        status::YES
                    } else {
                        status::NO
                    },
                );
            }
        }

        // Convert to porcelain format (same as on_show_models --porcelain)
        let porcelain_output = Porcelain::from(&info)
            .drop_col(0)
            .truncate(0, 40)
            .uppercase_headers();
        let porcelain_str = porcelain_output.to_string();

        // Split into header + data lines
        let all_lines: Vec<&str> = porcelain_str.lines().collect();
        if all_lines.is_empty() {
            return Ok(None);
        }

        // Build a flat list of (ModelId, ProviderId) for the data rows.
        // The first line is the header; data rows follow in the same order as
        // the Info entries (sorted by provider, then model within provider).
        let mut model_entries: Vec<(ModelId, ProviderId)> = Vec::new();
        for pm in &all_provider_models {
            for model in &pm.models {
                model_entries.push((model.id.clone(), pm.provider_id.clone()));
            }
        }

        let mut rows = Vec::with_capacity(all_lines.len());
        // Header row (non-selectable via header_lines=1)
        let Some(header) = all_lines.first() else {
            return Err(UIError::MissingHeaderLine.into());
        };
        rows.push(SelectRow::header(header.to_string()));
        // Data rows
        for (i, line) in all_lines.iter().skip(1).enumerate() {
            let Some((model_id, provider_id)) = model_entries.get(i) else {
                continue;
            };
            let dotted_id = model_id.as_str().replace(['-', '_'], ".");
            rows.push(SelectRow {
                raw: format!("{}\t{}", model_id.as_str(), provider_id.as_ref()),
                display: line.to_string(),
                search: format!(
                    "{} {} {} {}",
                    model_id.as_str(),
                    dotted_id,
                    provider_id.as_ref(),
                    line
                ),
                fields: vec![model_id.to_string(), provider_id.as_ref().to_string()],
            });
        }

        // Find starting cursor position for the current model.
        let current_model = self
            .get_agent_model(self.api.get_active_agent().await)
            .await;
        let current_provider = self
            .get_provider(self.api.get_active_agent().await)
            .await
            .ok()
            .map(|provider| provider.id);
        let initial_raw = current_model.as_ref().and_then(|current| {
            model_entries
                .iter()
                .find(|(model_id, provider_id)| {
                    model_id == current
                        && current_provider
                            .as_ref()
                            .map(|provider| provider_id == provider)
                            .unwrap_or(true)
                })
                .map(|(model_id, provider_id)| {
                    format!("{}\t{}", model_id.as_str(), provider_id.as_ref())
                })
        });

        let selected = self.select_raw_row("Model ❯ ", query, rows, 1, initial_raw)?;

        let Some(selected) = selected else {
            return Ok(None);
        };

        let mut parts = selected.raw.splitn(2, '\t');
        let selection = match (parts.next(), parts.next()) {
            (Some(model_id), Some(provider_id)) => Some((
                ModelId::new(model_id.to_string()),
                ProviderId::from(provider_id.to_string()),
            )),
            _ => None,
        };
        Ok(selection)
    }

    // Helper method to handle model selection and update the conversation.
    // When `provider_filter` is `Some`, only models from that provider are shown.
    // The model and provider returned by the selector are always set as one
    // atomic operation.
    #[async_recursion::async_recursion]
    pub(super) async fn on_model_selection(
        &mut self,
        provider_filter: Option<ProviderId>,
    ) -> Result<Option<ModelId>> {
        // Select a model; the selector returns both the model and its provider
        let selection = self.select_model(provider_filter, None).await?;

        // If no model was selected (user canceled), return early
        let (model, provider_id) = match selection {
            Some(pair) => pair,
            None => return Ok(None),
        };

        // Set model and provider atomically as a single config operation
        self.api
            .update_config(vec![ConfigOperation::SetSessionConfig(
                forge_domain::ModelConfig::new(provider_id, model.clone()),
            )])
            .await?;

        // Update the UI state with the new model
        self.update_model(Some(model.clone()));

        self.writeln_title(TitleFormat::action(format!("Switched to model: {model}")))?;

        Ok(Some(model))
    }

    /// Validates that a model exists, optionally scoped to a specific provider.
    /// When `provider` is `None`, models are fetched from the default provider.
    pub(super) async fn validate_model(
        &self,
        model_str: &str,
        provider: Option<&forge_domain::ProviderId>,
    ) -> Result<ModelId> {
        let models = match provider {
            None => self.api.get_models().await?,
            Some(provider_id) => {
                self.api
                    .get_all_provider_models()
                    .await?
                    .into_iter()
                    .find(|pm| &pm.provider_id == provider_id)
                    .with_context(|| {
                        format!("Provider '{provider_id}' not found or returned no models")
                    })?
                    .models
            }
        };
        let model_id = ModelId::new(model_str);
        models
            .iter()
            .find(|m| m.id == model_id)
            .map(|_| model_id)
            .with_context(|| {
                let hints = models
                    .iter()
                    .take(10)
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let suggestion = if models.len() > 10 {
                    format!("{hints} (and {} more)", models.len() - 10)
                } else {
                    hints
                };
                format!("Model '{model_str}' not found. Available models: {suggestion}")
            })
    }
}

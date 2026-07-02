mod chat;
mod config;
mod conversation;
mod listing;
mod mcp;
mod provider;
mod repl;
mod selection;
mod subcommands;
mod terminal;
mod workspace;

use std::sync::Arc;

use anyhow::{Context, Result};
use colored::Colorize;
use convert_case::{Case, Casing};
use forge_api::{API, AgentId, Conversation, ConversationId, ModelId, Provider};
use forge_config::ForgeConfig;
use forge_display::MarkdownFormat;
use forge_domain::{ConsoleWriter, TitleFormat};
use forge_fs::ForgeFS;
use forge_select::{ForgeWidget, SelectRow};
use forge_spinner::SpinnerManager;
use url::Url;

use crate::cli::Cli;
use crate::editor::ReadLineError;
use crate::input::Console;
use crate::model::{AppCommand, ForgeCommandManager};
use crate::prompt::ForgePrompt;
use crate::state::UIState;
use crate::stream_renderer::SharedSpinner;
use crate::title_display::TitleDisplayExt;
use crate::update::on_update;
use crate::{TRACKER, banner, tracker};

// File-specific constants
const MISSING_AGENT_TITLE: &str = "<missing agent.title>";

/// Conversation dump format used by the /dump command
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ConversationDump {
    conversation: Conversation,
    related_conversations: Vec<Conversation>,
}

pub struct UI<A: ConsoleWriter, F: Fn(ForgeConfig) -> A> {
    markdown: MarkdownFormat,
    state: UIState,
    api: Arc<F::Output>,
    new_api: Arc<F>,
    console: Console,
    command: Arc<ForgeCommandManager>,
    cli: Cli,
    spinner: SharedSpinner<A>,
    config: ForgeConfig,
    #[allow(dead_code)] // The guard is kept alive by being held in the struct
    _guard: forge_tracker::Guard,
}

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    /// Writes a line to the console output
    /// Takes anything that implements ToString trait
    fn writeln<T: ToString>(&mut self, content: T) -> anyhow::Result<()> {
        self.spinner.write_ln(content)
    }

    /// Writes a TitleFormat to the console output with proper formatting
    fn writeln_title(&mut self, title: TitleFormat) -> anyhow::Result<()> {
        self.spinner.write_ln(title.display())
    }

    fn writeln_to_stderr(&mut self, title: String) -> anyhow::Result<()> {
        self.spinner.ewrite_ln(title)
    }

    /// Helper to get provider for an optional agent, defaulting to the current
    /// active agent's provider
    async fn get_provider(&self, agent_id: Option<AgentId>) -> Result<Provider<Url>> {
        match agent_id {
            Some(agent_id) => self.api.get_agent_provider(agent_id).await,
            None => self.api.get_default_provider().await,
        }
    }

    /// Helper to get model for an optional agent, defaulting to the current
    /// active agent's model
    async fn get_agent_model(&self, agent_id: Option<AgentId>) -> Option<ModelId> {
        match agent_id {
            Some(agent_id) => self.api.get_agent_model(agent_id).await,
            None => self.api.get_session_config().await.map(|c| c.model),
        }
    }

    fn select_raw_row(
        &self,
        prompt: &str,
        query: Option<String>,
        rows: Vec<SelectRow>,
        header_lines: usize,
        initial_raw: Option<String>,
    ) -> Result<Option<SelectRow>> {
        ForgeWidget::select_rows(prompt, rows)
            .query(query)
            .header_lines(header_lines)
            .initial_raw(initial_raw)
            .prompt()
    }

    fn select_row_output(
        &mut self,
        prompt: &str,
        query: Option<String>,
        rows: Vec<SelectRow>,
    ) -> Result<()> {
        if let Some(row) = self.select_raw_row(prompt, query, rows, 1, None)? {
            self.writeln(row.raw)?;
        }

        Ok(())
    }

    fn porcelain_rows(porcelain: impl ToString) -> Result<Vec<SelectRow>> {
        let porcelain = porcelain.to_string();
        let mut lines = porcelain.lines();
        let Some(header) = lines.next() else {
            return Ok(Vec::new());
        };

        let mut rows = vec![SelectRow::header(header.to_string())];
        rows.extend(lines.filter_map(|line| {
            line.split_whitespace()
                .next()
                .map(|raw| SelectRow::new(raw.to_string(), line.to_string()))
        }));

        Ok(rows)
    }

    /// Displays banner only if user is in interactive mode.
    fn display_banner(&self) -> Result<()> {
        if self.cli.is_interactive() {
            banner::display(false)?;
        }
        Ok(())
    }

    // Handle creating a new conversation
    async fn on_new(&mut self) -> Result<()> {
        let config = forge_config::ForgeConfig::read().unwrap_or_default();
        self.config = config.clone();
        self.api = Arc::new((self.new_api)(config));
        self.init_state(false).await?;

        // Set agent if provided via CLI
        if let Some(agent_id) = self.cli.agent.clone() {
            self.api.set_active_agent(agent_id).await?;
        }

        // Reset previously set CLI parameters by the user
        self.cli.conversation = None;
        self.cli.conversation_id = None;

        self.spinner.reset();
        self.display_banner()?;
        self.trace_user();
        self.hydrate_caches();
        Ok(())
    }

    // Set the current mode and update conversation variable
    async fn on_agent_change(&mut self, agent_id: AgentId) -> Result<()> {
        // Convert string to AgentId for validation
        let agent = self
            .api
            .get_agent_infos()
            .await?
            .into_iter()
            .find(|info| info.id == agent_id)
            .ok_or(anyhow::anyhow!("Undefined agent: {agent_id}"))?;

        // Update the app config with the new operating agent.
        self.api.set_active_agent(agent.id.clone()).await?;

        // Update model tracking to reflect the new agent's model
        let model = self.get_agent_model(Some(agent.id.clone())).await;
        self.update_model(model.clone());

        let name = agent.id.as_str().to_case(Case::UpperSnake).bold();

        let title = format!(
            "∙ {}",
            agent.title.as_deref().unwrap_or(MISSING_AGENT_TITLE)
        )
        .dimmed();

        // Show model info if agent uses a specific model
        let model_info = model
            .map(|m| format!(" ∙ model: {m}").dimmed().to_string())
            .unwrap_or_default();

        self.writeln_title(TitleFormat::action(format!("{name} {title}{model_info}")))?;

        Ok(())
    }

    /// Initialises the UI with the provided CLI arguments and API factory.
    ///
    /// # Arguments
    /// * `cli` - Parsed command-line arguments
    /// * `config` - Pre-read application configuration for the initial API
    ///   instance
    /// * `f` - Factory closure invoked once at startup and again on each `/new`
    ///   command; receives the latest [`ForgeConfig`] so that config changes
    ///   from `forge config set` are reflected in new conversations
    pub fn init(cli: Cli, config: ForgeConfig, f: F) -> Result<Self> {
        // Parse CLI arguments first to get flags
        let api = Arc::new(f(config.clone()));
        let env = api.environment();
        let command = Arc::new(ForgeCommandManager::default());
        let spinner = SharedSpinner::new(SpinnerManager::new(api.clone()));
        Ok(Self {
            state: UIState::new(env.clone()),
            api,
            new_api: Arc::new(f),
            console: Console::new(
                env.clone(),
                config.custom_history_path.clone(),
                command.clone(),
            ),
            cli,
            command,
            spinner,
            markdown: MarkdownFormat::new(),
            config,
            _guard: forge_tracker::init_tracing(env.log_path(), TRACKER.clone())?,
        })
    }

    async fn prompt(&self) -> Result<AppCommand> {
        // Get usage from current conversation if available.
        // Use the last message's usage for token count (context window size),
        // but replace cost with the accumulated session cost so the cost
        // shown reflects the total spend rather than just the last request.
        let usage = if let Some(conversation_id) = &self.state.conversation_id {
            self.api
                .conversation(conversation_id)
                .await
                .ok()
                .flatten()
                .and_then(|conv| {
                    conv.usage().map(|mut u| {
                        u.cost = conv.accumulated_cost();
                        u
                    })
                })
        } else {
            None
        };

        // Prompt the user for input
        let agent_id = self.api.get_active_agent().await.unwrap_or_default();
        let model = self
            .get_agent_model(self.api.get_active_agent().await)
            .await;
        let reasoning_effort = self.api.get_reasoning_effort().await.ok().flatten();
        let mut forge_prompt = ForgePrompt::new(self.state.cwd.clone(), agent_id);
        if let Some(u) = usage {
            forge_prompt.usage(u);
        }
        if let Some(m) = model {
            forge_prompt.model(m);
        }
        if let Some(e) = reasoning_effort {
            forge_prompt.reasoning_effort(e);
        }
        self.console.prompt(&mut forge_prompt).await
    }

    pub async fn run(&mut self) {
        match self.run_inner().await {
            Ok(_) => {}
            Err(error) => {
                tracing::error!(error = ?error);

                // Display the full error chain for better debugging
                let mut error_message = error.to_string();
                let mut source = error.source();
                while let Some(err) = source {
                    error_message.push_str(&format!("\n    Caused by: {}", err));
                    source = err.source();
                }

                let _ =
                    self.writeln_to_stderr(TitleFormat::error(error_message).display().to_string());
            }
        }
    }

    async fn run_inner(&mut self) -> Result<()> {
        if let Some(cmd) = self.cli.subcommands.clone() {
            return self.handle_subcommands(cmd).await;
        }

        // Display the banner in dimmed colors since we're in interactive mode
        self.display_banner()?;
        self.init_state(true).await?;

        self.trace_user();
        self.hydrate_caches();
        self.init_conversation().await?;

        // Check for dispatch flag first
        if let Some(dispatch_json) = self.cli.event.clone() {
            return self.handle_dispatch(dispatch_json).await;
        }

        // Handle direct prompt or piped input if provided (raw text messages)
        let input = self.cli.prompt.clone().or(self.cli.piped_input.clone());
        if let Some(input) = input {
            tracker::prompt(input.clone());
            self.spinner.start(None)?;
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("User interrupted operation with Ctrl+C");
                    self.spinner.reset();
                    return Ok(());
                }
                result = self.on_message(Some(input)) => {
                    result?;
                }
            }
            return Ok(());
        }

        // Get initial input from prompt
        // Prompt can fail if it doesn't have access to TTY. If it fails the first time,
        // we will stop everything and bubble up the error.
        let mut command = self.prompt().await;

        loop {
            match command {
                Ok(command) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {
                            self.spinner.reset();
                            tracing::info!("User interrupted operation with Ctrl+C");
                        }
                        result = self.on_command(command) => {
                            match result {
                                Ok(exit) => if exit {return Ok(())},
                                Err(error) => {
                                    if let Some(conversation_id) = self.state.conversation_id.as_ref()
                                        && let Some(conversation) = self.api.conversation(conversation_id).await.ok().flatten() {
                                            TRACKER.set_conversation(conversation).await;
                                        }
                                    tracker::error(&error);
                                    tracing::error!(error = ?error);
                                    self.spinner.stop(None)?;
                                    self.writeln_to_stderr(TitleFormat::error(format!("{error:?}")).display().to_string())?;
                                },
                            }
                        }
                    }

                    self.spinner.stop(None)?;
                }
                Err(error) => {
                    tracker::error(&error);
                    tracing::error!(error = ?error);
                    self.spinner.stop(None)?;

                    match error.downcast::<ReadLineError>() {
                        Ok(error) => {
                            Err(error)?;
                        }
                        Err(error) => self.writeln_to_stderr(
                            TitleFormat::error(error.to_string()).display().to_string(),
                        )?,
                    }
                }
            }
            // Centralized prompt call at the end of the loop
            command = self.prompt().await;
        }
    }

    // Improve startup time by hydrating caches
    fn hydrate_caches(&self) {
        let api = self.api.clone();
        tokio::spawn(async move { api.get_models().await });
        let api = self.api.clone();
        tokio::spawn(async move { api.get_tools().await });
        let api = self.api.clone();
        tokio::spawn(async move { api.get_agent_infos().await });
        let api = self.api.clone();
        tokio::spawn(async move {
            let _ = api.hydrate_channel();
        });
    }

    /// Initializes and returns a conversation ID for the current session.
    ///
    /// Handles conversation setup for both interactive and headless modes:
    /// - **Interactive**: Reuses existing conversation, loads from file, or
    ///   creates new
    /// - **Headless**: Uses environment variables or generates new conversation
    ///
    /// Displays initialization status and updates UI state with the
    /// conversation ID.
    async fn init_conversation(&mut self) -> Result<ConversationId> {
        // Set agent if provided via CLI
        if let Some(agent_id) = self.cli.agent.clone() {
            self.api.set_active_agent(agent_id).await?;
        }

        let mut is_new = false;
        let id = if let Some(id) = self.state.conversation_id {
            id
        } else if let Some(id) = self.cli.conversation_id {
            // Use the provided conversation ID

            // Check if conversation exists, if not create it
            if self.api.conversation(&id).await?.is_none() {
                let conversation = Conversation::new(id);
                self.api.upsert_conversation(conversation).await?;
                is_new = true;
            }
            id
        } else if let Some(ref path) = self.cli.conversation {
            let content = ForgeFS::read_utf8(path).await?;

            // Try to parse as a dump file first (with "conversation" wrapper)
            let conversation: Conversation = if let Ok(dump) =
                serde_json::from_str::<ConversationDump>(&content)
            {
                dump.conversation
            } else {
                // Fall back to parsing as direct Conversation object
                serde_json::from_str(&content)
                    .context("Failed to parse conversation file. Expected either a ConversationDump or Conversation format")?
            };

            let id = conversation.id;
            self.api.upsert_conversation(conversation).await?;
            id
        } else {
            let conversation = Conversation::generate();
            let id = conversation.id;
            is_new = true;
            self.api.upsert_conversation(conversation).await?;
            id
        };

        // Print if the state is being reinitialized
        if self.state.conversation_id.is_none() {
            self.print_conversation_status(is_new, id)?;
        }

        // Always set the conversation id in state
        self.state.conversation_id = Some(id);

        Ok(id)
    }

    fn print_conversation_status(
        &mut self,
        new_conversation: bool,
        id: ConversationId,
    ) -> Result<(), anyhow::Error> {
        let mut title = if new_conversation {
            "Initialize".to_string()
        } else {
            "Continue".to_string()
        };

        title.push_str(format!(" {}", id.into_string()).as_str());

        self.writeln_title(TitleFormat::debug(title))?;
        Ok(())
    }

    /// Initialize the state of the UI
    async fn init_state(&mut self, first: bool) -> Result<()> {
        let _ = self.handle_migrate_credentials().await;

        // Ensure we have a model selected before proceeding with initialization
        let active_agent = self.api.get_active_agent().await;

        // Validate provider is configured before loading agents
        // If provider is set in config but not configured (no credentials), prompt user
        // to login
        if self.api.get_session_config().await.is_none() && !self.on_provider_selection().await? {
            return Ok(());
        }

        let mut operating_model = self.get_agent_model(active_agent.clone()).await;
        if operating_model.is_none() {
            // Use the model returned from selection instead of re-fetching
            operating_model = self.on_model_selection(None).await?;
        }

        if first {
            // For chat, we are trying to get active agent or setting it to default.
            // So for default values, `/info` doesn't show active provider, model, etc.
            // So my default, on new, we should set the active agent.
            self.api
                .set_active_agent(active_agent.clone().unwrap_or_default())
                .await?;
            // only call on_update if this is the first initialization
            on_update(self.api.clone(), self.config.updates.as_ref()).await;
            // Apply the MCP trust gate. Servers are NOT connected here —
            // connections remain lazy and happen on first tool use.
            self.api.init_mcp().await?;
        }

        // Execute independent operations in parallel to improve performance
        let (agents_result, commands_result) =
            tokio::join!(self.api.get_agent_infos(), self.api.get_commands());

        // Register agent commands with proper error handling and user feedback
        match agents_result {
            Ok(agents) => {
                let registration_result = self.command.register_agent_commands(agents);

                // Show warning for any skipped agents due to conflicts
                for skipped_command in registration_result.skipped_conflicts {
                    self.writeln_title(TitleFormat::error(format!(
                        "Skipped agent command '{skipped_command}' due to name conflict with built-in command"
                    )))?;
                }
            }
            Err(e) => {
                self.writeln_title(TitleFormat::error(format!(
                    "Failed to load agents for command registration: {e}"
                )))?;
            }
        }

        // Register all the commands
        self.command.register_all(commands_result?);

        self.state = UIState::new(self.api.environment());
        self.update_model(operating_model);

        Ok(())
    }

    fn update_model(&mut self, model: Option<ModelId>) {
        if let Some(ref model) = model {
            tracker::set_model(model.to_string());
        }
    }

    fn trace_user(&self) {
        let api = self.api.clone();
        // NOTE: Spawning required so that we don't block the user while querying user
        // info
        tokio::spawn(async move {
            if let Ok(Some(user_info)) = api.user_info().await {
                tracker::login(user_info.auth_provider_id.into_string());
            }
        });
    }

    /// Handle credential migration
    async fn handle_migrate_credentials(&mut self) -> Result<()> {
        // Perform the migration
        self.spinner.start(Some("Migrating credentials"))?;
        let result = self.api.migrate_env_credentials().await?;
        self.spinner.stop(None)?;

        // Display results based on whether migration occurred
        if let Some(result) = result {
            self.writeln_title(
                TitleFormat::warning("Forge no longer reads API keys from environment variables.")
                    .sub_title("Learn more: https://forgecode.dev/docs/custom-providers/"),
            )?;

            let count = result.migrated_providers.len();
            let message = if count == 1 {
                "Migrated 1 provider from environment variables".to_string()
            } else {
                format!("Migrated {count} providers from environment variables")
            };
            self.writeln_title(TitleFormat::info(message))?;
        }
        Ok(())
    }

    /// Silently install VS Code extension if in VS Code and extension not
    /// installed.
    /// NOTE: This is a non-cancellable and a slow task. We should only run this
    /// if the user has provided a prompt because that is guaranteed to run for
    /// at least a few seconds.
    fn install_vscode_extension(&self) {
        tokio::task::spawn_blocking(|| {
            if crate::vscode::should_install_extension() {
                let _ = crate::vscode::install_extension();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    // Note: Tests for confirm_delete_conversation are disabled because
    // ForgeSelect::confirm is not easily mockable in the current
    // architecture. The functionality is tested through integration tests
    // instead.
}

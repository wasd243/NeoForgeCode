use anyhow::Context;
use colored::Colorize;
use forge_api::{API, AgentId, ConversationId, UserPrompt};
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, TitleFormat};
use forge_select::ForgeWidget;

use super::UI;
use crate::cli::CommitCommandGroup;
use crate::info::Info;
use crate::model::AppCommand;
use crate::update::on_update;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    pub(super) async fn on_command(&mut self, command: AppCommand) -> anyhow::Result<bool> {
        match command {
            AppCommand::Conversations { id } => {
                if let Some(raw_id) = id {
                    let conversation_id = ConversationId::parse(&raw_id)
                        .context(format!("Invalid conversation ID: {raw_id}"))?;
                    let conversation = self.validate_conversation_exists(&conversation_id).await?;
                    self.state.conversation_id = Some(conversation_id);
                    self.on_show_last_message(conversation, false).await?;
                    self.writeln_title(TitleFormat::info(format!(
                        "Switched to conversation {}",
                        conversation_id.into_string().bold()
                    )))?;
                    self.on_info(false, Some(conversation_id)).await?;
                } else {
                    self.list_conversations().await?;
                }
            }
            AppCommand::ConversationTree => {
                let conversation_id = self
                    .state
                    .conversation_id
                    .ok_or_else(|| anyhow::anyhow!("No active conversation"))?;
                let parent = self.validate_conversation_exists(&conversation_id).await?;
                let children = self.fetch_related_conversations(&parent).await;

                if children.is_empty() {
                    self.writeln_title(TitleFormat::info("No child conversations found."))?;
                } else if let Some(conversation) =
                    crate::conversation_selector::ConversationSelector::select_conversation(
                        &children,
                        self.state.conversation_id,
                        None,
                    )
                    .await?
                {
                    let conversation_id = conversation.id;
                    self.state.conversation_id = Some(conversation_id);
                    self.on_show_last_message(conversation, false).await?;
                    self.writeln_title(TitleFormat::info(format!(
                        "Switched to conversation {}",
                        conversation_id.into_string().bold()
                    )))?;
                    self.on_info(false, Some(conversation_id)).await?;
                }
            }
            AppCommand::Compact => {
                self.spinner.start(Some("Compacting"))?;
                self.on_compaction().await?;
            }
            AppCommand::Delete => {
                self.handle_delete_conversation().await?;
            }
            AppCommand::Rename { ref name } => {
                self.handle_rename_conversation(name.join(" ")).await?;
            }
            AppCommand::Dump { html, .. } => {
                self.spinner.start(Some("Dumping"))?;
                self.on_dump(html).await?;
            }
            AppCommand::New => {
                self.on_new().await?;
            }
            AppCommand::Info => {
                self.on_info(false, self.state.conversation_id).await?;
            }
            AppCommand::Usage => {
                self.on_usage().await?;
            }
            AppCommand::Message(ref content) => {
                self.spinner.start(None)?;
                self.on_message(Some(content.clone())).await?;
            }
            AppCommand::Forge => {
                self.on_agent_change(AgentId::FORGE).await?;
            }
            AppCommand::Muse => {
                self.on_agent_change(AgentId::MUSE).await?;
            }
            AppCommand::Sage => {
                self.on_agent_change(AgentId::SAGE).await?;
            }
            AppCommand::Help => {
                let info = Info::from(self.command.as_ref());
                self.writeln(info)?;
            }
            AppCommand::Tools => {
                let agent_id = self.api.get_active_agent().await.unwrap_or_default();
                self.on_show_tools(agent_id, false).await?;
            }
            AppCommand::Update => {
                on_update(self.api.clone(), None).await;
            }
            AppCommand::Exit => {
                return Ok(true);
            }

            AppCommand::Custom(event) => {
                self.spinner.start(None)?;
                self.on_custom_event(event.into()).await?;
            }
            AppCommand::Model => {
                self.on_model_selection(None).await?;
            }
            AppCommand::Shell(ref command) => {
                self.api.execute_shell_command_raw(command).await?;
            }
            AppCommand::Commit { max_diff_size, .. } => {
                let args = CommitCommandGroup {
                    preview: false,
                    max_diff_size: max_diff_size.or(Some(100_000)),
                    diff: None,
                    text: Vec::new(),
                };
                let result = self.handle_commit_command(args).await?;
                if !result.git_output.is_empty() {
                    self.writeln(result.git_output.trim_end())?;
                } else {
                    self.writeln(result.message.trim_end())?;
                }
            }
            AppCommand::Agent => {
                if let Some(selected_agent) = self.select_agent(None).await? {
                    self.on_agent_change(selected_agent).await?;
                }
            }
            AppCommand::Login => {
                self.handle_provider_login(None).await?;
            }
            AppCommand::Logout => {
                return self.handle_provider_logout(None).await;
            }
            AppCommand::Retry => {
                self.spinner.start(None)?;
                self.on_message(None).await?;
            }
            AppCommand::Index => {
                let working_dir = self.state.cwd.clone();
                self.on_index(working_dir, false).await?;
            }
            AppCommand::AgentSwitch(agent_id) => {
                // Validate that the agent exists by checking against loaded agents
                let agents = self.api.get_agent_infos().await?;
                let agent_exists = agents.iter().any(|agent| agent.id.as_str() == agent_id);

                if agent_exists {
                    self.on_agent_change(AgentId::new(agent_id)).await?;
                } else {
                    return Err(anyhow::anyhow!(
                        "Agent '{agent_id}' not found or unavailable"
                    ));
                }
            }
            AppCommand::Config => {
                self.on_show_config(false).await?;
            }
            AppCommand::ConfigModel => {
                self.on_model_selection(None).await?;
            }
            AppCommand::ConfigReload => {
                self.writeln_title(TitleFormat::info(
                    "No session overrides in REPL mode. Use :model to switch the active model.",
                ))?;
            }
            AppCommand::Effort => {
                self.on_reasoning_effort_selection(false).await?;
            }
            AppCommand::ConfigReasoningEffort => {
                self.on_reasoning_effort_selection(true).await?;
            }
            AppCommand::CommitModel => {
                self.on_config_commit_model().await?;
            }
            AppCommand::ConfigSuggestModel => {
                self.on_config_suggest_model().await?;
            }
            AppCommand::ConfigEdit => {
                self.on_config_edit().await?;
            }
            AppCommand::Skill => {
                self.on_show_skills(false, false).await?;
            }
            AppCommand::Edit { content } => {
                let initial = if content.is_empty() {
                    None
                } else {
                    Some(content.join(" ").trim().to_string())
                };
                self.on_edit_buffer(initial).await?;
            }
            AppCommand::CommitPreview => {
                let args = CommitCommandGroup {
                    preview: true,
                    max_diff_size: Some(100_000),
                    diff: None,
                    text: Vec::new(),
                };
                let result = self.handle_commit_command(args).await?;
                let flags = if result.has_staged_files { "" } else { " -a" };
                let commit_command = format!("!git commit{flags} -m '{}'", result.message);
                self.console.set_buffer(commit_command);
            }
            AppCommand::Suggest { description } => {
                let desc = if description.is_empty() {
                    None
                } else {
                    Some(description.join(" ").trim().to_string())
                };
                self.on_suggest(desc).await?;
            }
            AppCommand::Clone { id } => {
                self.on_slash_clone(id).await?;
            }
            AppCommand::ConversationRename { name } => {
                let args = if name.is_empty() {
                    None
                } else {
                    Some(name.join(" ").trim().to_string())
                };
                self.on_slash_conversation_rename(args).await?;
            }
            AppCommand::Copy => {
                self.on_copy().await?;
            }
            AppCommand::WorkspaceSync => {
                let working_dir = self.state.cwd.clone();
                self.on_index(working_dir, true).await?;
            }
            AppCommand::WorkspaceStatus => {
                let cwd = self.state.cwd.clone();
                self.on_workspace_status(cwd, false).await?;
            }
            AppCommand::WorkspaceInfo => {
                let cwd = self.state.cwd.clone();
                self.on_workspace_info(cwd).await?;
            }
            AppCommand::WorkspaceInit => {
                let cwd = self.state.cwd.clone();
                self.on_workspace_init(cwd, false).await?;
            }
        }

        Ok(false)
    }

    /// Handle the cmd command - generates shell command from natural language
    pub(super) async fn on_cmd(&mut self, prompt: UserPrompt) -> anyhow::Result<()> {
        self.spinner.start(Some("Generating"))?;

        match self.api.generate_command(prompt).await {
            Ok(command) => {
                self.spinner.stop(None)?;
                self.writeln(command)?;
                Ok(())
            }
            Err(err) => {
                self.spinner.stop(None)?;
                Err(err)
            }
        }
    }

    /// Opens an external editor to compose a prompt and sets it in the REPL
    /// buffer on exit.
    ///
    /// # Arguments
    /// * `initial` - Optional text to pre-populate the editor with.
    pub(super) async fn on_edit_buffer(&mut self, initial: Option<String>) -> anyhow::Result<()> {
        use std::io::Write as _;

        let editor = std::env::var("FORGE_EDITOR")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "nano".to_string());

        // Split the editor string into binary + pre-configured flags
        // (e.g. "code --wait" → binary="code", extra_args=["--wait"])
        let mut editor_parts = editor.split_whitespace();
        let editor_binary = editor_parts.next().unwrap_or("nano").to_string();
        let editor_flags: Vec<&str> = editor_parts.collect();

        // Create .forge directory for the temp file
        let forge_dir = self.state.cwd.join(".forge");
        std::fs::create_dir_all(&forge_dir)?;
        let temp_file = forge_dir.join("FORGE_EDITMSG.md");

        // Write initial content
        let mut file = std::fs::File::create(&temp_file)?;
        if let Some(text) = initial {
            file.write_all(text.as_bytes())?;
        }
        drop(file);

        let status = std::process::Command::new(&editor_binary)
            .args(&editor_flags)
            .arg(&temp_file)
            .status()
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to open editor '{}': {}. Set FORGE_EDITOR or EDITOR.",
                    editor_binary,
                    e
                )
            })?;

        if !status.success() {
            return Err(anyhow::anyhow!("Editor exited with error code: {}", status));
        }

        let content = std::fs::read_to_string(&temp_file)?;
        let content = content.trim().to_string();

        if content.is_empty() {
            self.writeln_title(TitleFormat::info("Editor closed with no content"))?;
            return Ok(());
        }

        // Pre-fill the REPL buffer so the user can review/edit before sending
        self.console.set_buffer(content);

        Ok(())
    }

    /// Generates a shell command from a natural language description and sets
    /// it in the REPL buffer.
    ///
    /// # Arguments
    /// * `description` - Optional natural language description. If `None`, an
    ///   interactive prompt is shown.
    pub(super) async fn on_suggest(&mut self, description: Option<String>) -> anyhow::Result<()> {
        let description = match description {
            Some(d) if !d.is_empty() => d,
            _ => {
                let input = ForgeWidget::input("Describe the command you want")
                    .allow_empty(false)
                    .prompt()?;
                match input {
                    Some(d) if !d.is_empty() => d,
                    _ => {
                        self.writeln_title(TitleFormat::error(
                            "No description provided. Usage: :suggest <description>",
                        ))?;
                        return Ok(());
                    }
                }
            }
        };

        self.spinner.start(Some("Generating"))?;

        let prompt = UserPrompt::from(description);
        match self.api.generate_command(prompt).await {
            Ok(command) => {
                self.spinner.stop(None)?;
                self.writeln(command.clone())?;
                // Set the generated command in the buffer for review
                self.console.set_buffer(command);
                Ok(())
            }
            Err(err) => {
                self.spinner.stop(None)?;
                Err(err)
            }
        }
    }

    /// Copies the last AI response from the active conversation to the
    /// system clipboard.
    pub(super) async fn on_copy(&mut self) -> anyhow::Result<()> {
        let conversation_id = match &self.state.conversation_id {
            Some(cid) => *cid,
            None => {
                self.writeln_title(TitleFormat::error(
                    "No active conversation. Start a conversation first.",
                ))?;
                return Ok(());
            }
        };

        let conversation = match self.api.conversation(&conversation_id).await? {
            Some(conv) => conv,
            None => {
                self.writeln_title(TitleFormat::error("Conversation not found."))?;
                return Ok(());
            }
        };

        let context = match &conversation.context {
            Some(ctx) => ctx.clone(),
            None => {
                self.writeln_title(TitleFormat::error("Conversation has no messages."))?;
                return Ok(());
            }
        };

        // Find the last assistant message
        let content = context.messages.iter().rev().find_map(|msg| match &**msg {
            forge_domain::ContextMessage::Text(forge_api::TextMessage {
                content,
                role: forge_domain::Role::Assistant,
                ..
            }) => Some(content.clone()),
            _ => None,
        });

        match content {
            None => {
                self.writeln_title(TitleFormat::error(
                    "No assistant message found in this conversation.",
                ))?;
            }
            Some(content) => {
                // I don't know why they're going to support android target, this is unnecessary, and
                // nobody will code on their phones.
                // I've let my agent refactor this 5000+ lines shit
                // NO MORE TELEPHONE SUPPORT IN NEOFORGECODE
                #[cfg(not(target_os = "android"))]
                let _copied = arboard::Clipboard::new()
                    .and_then(|mut cb| cb.set_text(content.clone()))
                    .is_ok();
            }
        }

        Ok(())
    }
}

use anyhow::{Context, Result};
use forge_api::{API, UserPrompt};
use forge_app::CommitResult;
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, TitleFormat};
use tokio_stream::StreamExt;

use super::UI;
use super::mcp::format_mcp_server;
use crate::cli::{CommitCommandGroup, ListCommand, McpCommand, SelectCommand, TopLevelCommand};
use crate::conversation_selector::ConversationSelector;
use crate::display_constants::markers;
use crate::info::Info;
use crate::porcelain::Porcelain;
use crate::update::on_update;
use crate::banner;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    pub(super) async fn handle_generate_conversation_id(&mut self) -> Result<()> {
        let conversation_id = forge_domain::ConversationId::generate();
        println!("{}", conversation_id.into_string());
        Ok(())
    }

    pub(super) async fn handle_subcommands(
        &mut self,
        subcommand: TopLevelCommand,
    ) -> anyhow::Result<()> {
        match subcommand {
            TopLevelCommand::Agent(agent_group) => {
                match agent_group.command {
                    crate::cli::AgentCommand::List => {
                        self.on_show_agents(agent_group.porcelain, false).await?;
                    }
                }
                return Ok(());
            }
            TopLevelCommand::List(list_group) => {
                let porcelain = list_group.porcelain;
                match list_group.command {
                    ListCommand::Agent { custom } => {
                        self.on_show_agents(porcelain, custom).await?;
                    }
                    ListCommand::Provider { types } => {
                        self.on_show_providers(porcelain, types).await?;
                    }
                    ListCommand::Model => {
                        self.on_show_models(porcelain).await?;
                    }
                    ListCommand::Command { custom } => {
                        if custom {
                            self.on_show_custom_commands(porcelain).await?;
                        } else {
                            self.on_show_commands(porcelain).await?;
                        }
                    }
                    ListCommand::Config => {
                        self.on_show_config(porcelain).await?;
                    }
                    ListCommand::Tool { agent } => {
                        self.on_show_tools(agent, porcelain).await?;
                    }
                    ListCommand::Mcp => {
                        self.on_show_mcp_servers(porcelain).await?;
                    }
                    ListCommand::Conversation { parent } => {
                        if let Some(parent_id) = parent {
                            let parent_conv = self.validate_conversation_exists(&parent_id).await?;
                            let children = self.fetch_related_conversations(&parent_conv).await;

                            if children.is_empty() {
                                self.writeln_title(TitleFormat::info(
                                    "No child conversations found.",
                                ))?;
                            } else {
                                let mut info = Info::new();
                                for conv in children.into_iter() {
                                    let title = conv
                                        .title
                                        .as_deref()
                                        .map(|t| t.to_string())
                                        .unwrap_or_else(|| markers::EMPTY.to_string());

                                    let duration = chrono::Utc::now().signed_duration_since(
                                        conv.metadata
                                            .updated_at
                                            .unwrap_or(conv.metadata.created_at),
                                    );
                                    let duration = std::time::Duration::from_secs(
                                        (duration.num_minutes() * 60).max(0) as u64,
                                    );
                                    let time_ago = if duration.is_zero() {
                                        "now".to_string()
                                    } else {
                                        format!("{} ago", humantime::format_duration(duration))
                                    };

                                    info = info
                                        .add_title(conv.id)
                                        .add_key_value("Title", title)
                                        .add_key_value("Updated", time_ago);
                                }

                                let porcelain = Porcelain::from(&info)
                                    .drop_col(3)
                                    .truncate(1, 60)
                                    .uppercase_headers();
                                self.writeln(porcelain)?;
                            }
                        } else {
                            self.on_show_conversations(porcelain).await?;
                        }
                    }
                    ListCommand::Cmd => {
                        self.on_show_custom_commands(porcelain).await?;
                    }
                    ListCommand::Skill { custom } => {
                        self.on_show_skills(porcelain, custom).await?;
                    }
                    ListCommand::File => {
                        self.on_list_files(porcelain).await?;
                    }
                }
                return Ok(());
            }
            TopLevelCommand::Zsh(terminal_group) => {
                match terminal_group {
                    crate::cli::ZshCommandGroup::Plugin => {
                        self.on_zsh_plugin().await?;
                    }
                    crate::cli::ZshCommandGroup::Theme => {
                        self.on_zsh_theme().await?;
                    }
                    crate::cli::ZshCommandGroup::Doctor => {
                        self.on_zsh_doctor().await?;
                    }
                    crate::cli::ZshCommandGroup::Rprompt => {
                        if let Some(text) = self.handle_zsh_rprompt_command().await {
                            print!("{}", text)
                        }
                        return Ok(());
                    }
                    crate::cli::ZshCommandGroup::Setup => {
                        self.on_zsh_setup().await?;
                    }
                    crate::cli::ZshCommandGroup::Keyboard => {
                        self.on_zsh_keyboard().await?;
                    }
                    crate::cli::ZshCommandGroup::Format { buffer } => {
                        print!("{}", crate::zsh::paste::wrap_pasted_text(&buffer));
                        return Ok(());
                    }
                }
                return Ok(());
            }
            TopLevelCommand::Mcp(mcp_command) => match mcp_command.command {
                McpCommand::Import(import_args) => {
                    let scope: forge_domain::Scope = import_args.scope.into();

                    // Parse the incoming MCP configuration
                    let incoming_config: forge_domain::McpConfig = serde_json::from_str(&import_args.json)
                        .context("Failed to parse MCP configuration JSON. Expected format: {\"mcpServers\": {...}}")?;

                    // Read only the scope-specific config (not merged)
                    let mut scope_config = self.api.read_mcp_config(Some(&scope)).await?;

                    // Merge the incoming servers with scope-specific config only
                    let mut added_servers = Vec::new();
                    for (server_name, server_config) in incoming_config.mcp_servers {
                        scope_config
                            .mcp_servers
                            .insert(server_name.clone(), server_config);
                        added_servers.push(server_name);
                    }

                    // Write back to the specific scope only
                    self.api.write_mcp_config(&scope, &scope_config).await?;

                    // Log each added server after successful write
                    for server_name in added_servers {
                        self.writeln_title(TitleFormat::info(format!(
                            "Added MCP server '{server_name}'"
                        )))?;
                    }
                }
                McpCommand::List => {
                    self.on_show_mcp_servers(mcp_command.porcelain).await?;
                }
                McpCommand::Remove(rm) => {
                    let name = forge_api::ServerName::from(rm.name);
                    let scope: forge_domain::Scope = rm.scope.into();

                    // Read only the scope-specific config (not merged)
                    let mut scope_config = self.api.read_mcp_config(Some(&scope)).await?;

                    // Remove the server from scope-specific config only
                    scope_config.mcp_servers.remove(&name);

                    // Write back to the specific scope only
                    self.api.write_mcp_config(&scope, &scope_config).await?;

                    self.writeln_title(TitleFormat::info(format!("Removed server: {name}")))?;
                }
                McpCommand::Show(val) => {
                    let name = forge_api::ServerName::from(val.name);
                    let config = self.api.read_mcp_config(None).await?;
                    let server = config
                        .mcp_servers
                        .get(&name)
                        .ok_or(anyhow::anyhow!("Server not found"))?;

                    // Get MCP servers to check for failures
                    let tools = self.api.get_tools().await?;

                    // Display server configuration
                    self.writeln_title(TitleFormat::info(format!(
                        "{name}: {}",
                        format_mcp_server(server)
                    )))?;

                    // Display error if the server failed to initialize
                    if let Some(error) = tools.mcp.get_failures().get(&name) {
                        self.writeln_title(TitleFormat::error(error))?;
                    }
                }
                McpCommand::Reload => {
                    self.spinner.start(Some("Reloading MCPs"))?;
                    self.api.reload_mcp().await?;
                    self.writeln_title(TitleFormat::info("MCP reloaded"))?;
                }
                McpCommand::Login(args) => {
                    self.handle_mcp_login(&args.name).await?;
                }
                McpCommand::Logout(args) => {
                    self.handle_mcp_logout(&args.name).await?;
                }
            },
            TopLevelCommand::Info { porcelain, conversation_id } => {
                // Only initialize state (agent/provider/model resolution).
                // Avoid on_new() which also spawns fire-and-forget background
                // tasks via hydrate_caches() that race with process exit and
                // cause "JoinHandle polled after completion" panics.
                self.init_state(false).await?;

                self.on_info(porcelain, conversation_id).await?;
                return Ok(());
            }
            TopLevelCommand::Banner => {
                banner::display(true)?;
                return Ok(());
            }
            TopLevelCommand::Config(config_group) => {
                self.handle_config_command(config_group.command.clone(), config_group.porcelain)
                    .await?;
                return Ok(());
            }
            TopLevelCommand::Provider(provider_group) => {
                self.handle_provider_command(provider_group).await?;
                return Ok(());
            }
            TopLevelCommand::Conversation(conversation_group) => {
                self.handle_conversation_command(conversation_group).await?;
                return Ok(());
            }
            TopLevelCommand::Suggest { prompt } => {
                self.on_cmd(UserPrompt::from(prompt)).await?;
                return Ok(());
            }
            TopLevelCommand::Cmd(run_group) => {
                let porcelain = run_group.porcelain;
                match run_group.command {
                    crate::cli::CmdCommand::List { custom } => {
                        if custom {
                            self.on_show_custom_commands(porcelain).await?;
                        } else {
                            self.on_show_commands(porcelain).await?;
                        }
                    }
                    crate::cli::CmdCommand::Execute { commands: args } => {
                        // Execute the custom command
                        self.init_state(false).await?;

                        // If conversation_id is provided, set it in CLI before initializing
                        if let Some(ref cid) = run_group.conversation_id {
                            self.cli.conversation_id = Some(*cid);
                        }

                        self.init_conversation().await?;
                        self.spinner.start(None)?;

                        // Join all args into a single command string
                        let command_str = args.join(" ");

                        // Add slash prefix if not present
                        let command_with_slash = if command_str.starts_with('/') {
                            command_str
                        } else {
                            format!("/{command_str}")
                        };
                        let command = self.command.parse(&command_with_slash)?;
                        self.on_command(command).await?;
                    }
                }
                return Ok(());
            }
            TopLevelCommand::Workspace(index_group) => {
                match index_group.command {
                    crate::cli::WorkspaceCommand::Sync { path, init } => {
                        self.on_index(path, init).await?;
                    }
                    crate::cli::WorkspaceCommand::List { porcelain } => {
                        self.on_list_workspaces(porcelain).await?;
                    }
                    crate::cli::WorkspaceCommand::Query {
                        query,
                        path,
                        limit,
                        top_k,
                        use_case,
                        starts_with,
                        ends_with,
                    } => {
                        let mut params =
                            forge_domain::SearchParams::new(&query, &use_case).limit(limit);
                        if let Some(k) = top_k {
                            params = params.top_k(k);
                        }
                        if let Some(prefix) = starts_with {
                            params = params.starts_with(prefix);
                        }
                        if let Some(suffix) = ends_with {
                            params = params.ends_with(vec![suffix]);
                        }
                        self.on_query(path, params).await?;
                    }

                    crate::cli::WorkspaceCommand::Info { path } => {
                        self.on_workspace_info(path).await?;
                    }
                    crate::cli::WorkspaceCommand::Delete { workspace_ids } => {
                        self.on_delete_workspaces(workspace_ids).await?;
                    }
                    crate::cli::WorkspaceCommand::Status { path, porcelain } => {
                        self.on_workspace_status(path, porcelain).await?;
                    }
                    crate::cli::WorkspaceCommand::Init { path, yes } => {
                        self.on_workspace_init(path, yes).await?;
                    }
                }
                return Ok(());
            }
            TopLevelCommand::Commit(commit_group) => {
                self.init_state(false).await?;
                let preview = commit_group.preview;
                let result = self.handle_commit_command(commit_group).await?;
                if preview {
                    self.writeln(&result.message)?;
                } else if !result.git_output.is_empty() {
                    self.writeln_to_stderr(result.git_output.trim_end().to_string())?;
                } else {
                    self.writeln_to_stderr(result.message.trim_end().to_string())?;
                }
                return Ok(());
            }
            TopLevelCommand::Data(data_command_group) => {
                let mut stream = self.api.generate_data(data_command_group.into()).await?;
                while let Some(data) = stream.next().await {
                    self.writeln(data?)?;
                }
            }
            TopLevelCommand::Vscode(vscode_command) => {
                match vscode_command {
                    crate::cli::VscodeCommand::InstallExtension => {
                        self.on_vscode_extension_install().await?;
                    }
                }
                return Ok(());
            }
            TopLevelCommand::Update(args) => {
                let update = forge_config::Update::default().auto_update(args.no_confirm);
                on_update(self.api.clone(), Some(&update)).await;
                return Ok(());
            }
            TopLevelCommand::Setup => {
                self.on_zsh_setup().await?;
                return Ok(());
            }
            TopLevelCommand::Doctor => {
                self.on_zsh_doctor().await?;
                return Ok(());
            }
            TopLevelCommand::Logs(args) => {
                let log_dir = self.api.environment().log_path();
                crate::logs::run(args, log_dir).await?;
                return Ok(());
            }
            TopLevelCommand::Select(cmd) => {
                if !matches!(&cmd.command, SelectCommand::File { .. }) {
                    self.init_state(false).await?;
                }

                match &cmd.command {
                    SelectCommand::File { query } => {
                        if let Some(file) =
                            crate::completer::select_workspace_file(&self.state.cwd, query.clone())?
                        {
                            self.writeln(file)?;
                        }
                    }
                    SelectCommand::Model { query } => {
                        if let Some((model_id, provider_id)) =
                            self.select_model(None, query.clone()).await?
                        {
                            self.writeln(model_id.as_str())?;
                            self.writeln(provider_id.as_ref())?;
                        }
                    }
                    SelectCommand::Agent { query } => {
                        if let Some(agent_id) = self.select_agent(query.clone()).await? {
                            self.writeln(agent_id.as_str())?;
                        }
                    }
                    SelectCommand::Provider { query, configured } => {
                        if let Some(provider) =
                            self.select_provider(query.clone(), *configured).await?
                        {
                            self.writeln(provider.id().as_ref())?;
                        }
                    }
                    SelectCommand::ReasoningEffort { query } => {
                        if let Some(effort) = self
                            .select_reasoning_effort("Reasoning Effort", query.clone())
                            .await?
                        {
                            self.writeln(effort)?;
                        }
                    }
                    SelectCommand::Command { query } => {
                        let rows = Self::porcelain_rows(self.commands_porcelain().await?)?;

                        if !rows.is_empty() {
                            self.select_row_output("Command", query.clone(), rows)?;
                        }
                    }
                    SelectCommand::Conversation { query, parent } => {
                        let conversations = if let Some(parent_id) = parent {
                            let parent_conv = self.validate_conversation_exists(parent_id).await?;
                            self.fetch_related_conversations(&parent_conv).await
                        } else {
                            let max_conversations = self.config.max_conversations;
                            let conversations =
                                self.api.get_conversations(Some(max_conversations)).await?;
                            Self::user_initiated_conversations(conversations)
                        };

                        if !conversations.is_empty()
                            && let Some(conversation) = ConversationSelector::select_conversation(
                                &conversations,
                                self.state.conversation_id,
                                query.clone(),
                            )
                            .await?
                        {
                            self.writeln(conversation.id)?;
                        }
                    }
                }
                return Ok(());
            }
        }
        Ok(())
    }

    pub(super) async fn handle_commit_command(
        &mut self,
        commit_group: CommitCommandGroup,
    ) -> anyhow::Result<CommitResult> {
        self.spinner.start(Some("Creating commit"))?;

        // Convert Vec<String> to Option<String> by joining with spaces
        let additional_context = if commit_group.text.is_empty() {
            None
        } else {
            Some(commit_group.text.join(" "))
        };

        // Handle the commit command
        let result = self
            .api
            .commit(
                commit_group.preview,
                commit_group.max_diff_size,
                commit_group.diff,
                additional_context,
            )
            .await;

        match result {
            Ok(result) => {
                self.spinner.stop(None)?;
                Ok(result)
            }
            Err(e) => {
                self.spinner.stop(None)?;
                Err(e)
            }
        }
    }
}

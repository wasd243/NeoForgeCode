use std::sync::{Arc, Mutex};

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use forge_api::{AgentInfo, Model, Template};
use forge_domain::UserCommand;
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{EnumIter, EnumProperty};

use crate::info::Info;

/// Top-level Clap parser used to dispatch slash/colon commands.
///
/// The sentinel character (`/` or `:`) is stripped before passing tokens here,
/// so Clap only sees the subcommand name and its arguments.
#[derive(Debug, Parser)]
#[command(
    name = "forge_cmd",
    no_binary_name = true,
    disable_help_subcommand = true
)]
struct ClapCmd {
    #[command(subcommand)]
    sub: AppCommand,
}

/// Result of agent command registration
#[derive(Debug, Clone)]
pub struct AgentCommandRegistrationResult {
    pub registered_count: usize,
    pub skipped_conflicts: Vec<String>,
}

fn humanize_context_length(length: u64) -> String {
    if length >= 1_000_000 {
        format!("{:.1}M context", length as f64 / 1_000_000.0)
    } else if length >= 1_000 {
        format!("{:.1}K context", length as f64 / 1_000.0)
    } else {
        format!("{length} context")
    }
}

impl From<&[Model]> for Info {
    fn from(models: &[Model]) -> Self {
        let mut info = Info::new();

        for model in models.iter() {
            if let Some(context_length) = model.context_length {
                info = info.add_key_value(&model.id, humanize_context_length(context_length));
            } else {
                info = info.add_value(model.id.as_str());
            }
        }

        info
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeCommand {
    pub name: String,
    pub description: String,
    pub value: Option<String>,
}

#[derive(Debug)]
pub struct ForgeCommandManager {
    commands: Arc<Mutex<Vec<ForgeCommand>>>,
}

impl Default for ForgeCommandManager {
    fn default() -> Self {
        let commands = Self::default_commands();
        ForgeCommandManager { commands: Arc::new(Mutex::new(commands)) }
    }
}

impl ForgeCommandManager {
    /// Sanitizes agent ID to create a valid command name
    /// Replaces spaces and special characters with hyphens
    fn sanitize_agent_id(agent_id: &str) -> String {
        agent_id
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join("-")
    }

    /// Checks if a command name conflicts with built-in commands
    fn is_reserved_command(name: &str) -> bool {
        matches!(
            name,
            "agent"
                | "forge"
                | "muse"
                | "sage"
                | "help"
                | "compact"
                | "new"
                | "info"
                | "usage"
                | "exit"
                | "update"
                | "dump"
                | "model"
                | "tools"
                | "provider"
                | "login"
                | "logout"
                | "retry"
                | "conversations"
                | "conversation-tree"
                | "ct"
                | "list"
                | "commit"
                | "rename"
                | "rn"
                | "config"
                | "env"
                | "config-model"
                | "cm"
                | "config-reload"
                | "cr"
                | "model-reset"
                | "mr"
                | "reasoning-effort"
                | "re"
                | "config-reasoning-effort"
                | "cre"
                | "config-commit-model"
                | "ccm"
                | "config-suggest-model"
                | "csm"
                | "config-edit"
                | "ce"
                | "skill"
                | "edit"
                | "ed"
                | "commit-preview"
                | "suggest"
                | "s"
                | "clone"
                | "conversation-rename"
                | "copy"
                | "workspace-sync"
                | "sync"
                | "workspace-status"
                | "sync-status"
                | "workspace-info"
                | "sync-info"
                | "workspace-init"
                | "sync-init"
        )
    }

    fn default_commands() -> Vec<ForgeCommand> {
        AppCommand::iter()
            .filter(|command| !command.is_internal())
            .map(|command| ForgeCommand {
                name: command.name().to_string(),
                description: command.usage().to_string(),
                value: None,
            })
            .collect::<Vec<_>>()
    }

    /// Registers workflow commands from the API.
    pub fn register_all(&self, commands: Vec<forge_domain::Command>) {
        let mut guard = self.commands.lock().unwrap();

        // Remove existing workflow commands (those with ⚙ prefix in description)
        guard.retain(|cmd| !cmd.description.starts_with("⚙ "));

        // Add new workflow commands
        let new_commands = commands.into_iter().map(|cmd| {
            let name = cmd.name.clone();
            let description = format!("⚙ {}", cmd.description);
            let value = cmd.prompt.clone();

            ForgeCommand { name, description, value }
        });

        guard.extend(new_commands);

        // Sort commands for consistent completion behavior
        guard.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Registers agent commands to the manager.
    /// Returns information about the registration process.
    pub fn register_agent_commands(
        &self,
        agents: Vec<AgentInfo>,
    ) -> AgentCommandRegistrationResult {
        let mut guard = self.commands.lock().unwrap();
        let mut result =
            AgentCommandRegistrationResult { registered_count: 0, skipped_conflicts: Vec::new() };

        // Remove existing agent commands (commands starting with "agent-")
        guard.retain(|cmd| !cmd.name.starts_with("agent-"));

        // Add new agent commands
        for agent in agents {
            let agent_id_str = agent.id.as_str();
            let sanitized_id = Self::sanitize_agent_id(agent_id_str);
            let command_name = format!("agent-{sanitized_id}");

            // Skip if it would conflict with reserved commands
            if Self::is_reserved_command(&command_name) {
                result.skipped_conflicts.push(command_name);
                continue;
            }

            let default_title = agent_id_str.to_string();
            let title = agent.title.as_ref().unwrap_or(&default_title);
            let description = format!("🤖 Switch to {title} agent");

            guard.push(ForgeCommand {
                name: command_name,
                description,
                value: Some(agent_id_str.to_string()),
            });

            result.registered_count += 1;
        }

        // Sort commands for consistent completion behavior
        guard.sort_by(|a, b| a.name.cmp(&b.name));

        result
    }

    /// Finds a command by name.
    fn find(&self, command: &str) -> Option<ForgeCommand> {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.name == command)
            .cloned()
    }

    /// Lists all registered commands.
    pub fn list(&self) -> Vec<ForgeCommand> {
        self.commands.lock().unwrap().clone()
    }

    /// Extracts the command value from the input parts
    ///
    /// # Arguments
    /// * `command` - The command for which to extract the value
    /// * `parts` - The parts of the command input after the command name
    ///
    /// # Returns
    /// * `Option<String>` - The extracted value, if any
    fn extract_command_value(&self, command: &ForgeCommand, parts: &[&str]) -> Option<String> {
        // Unit tests implemented in the test module below

        // Try to get value provided in the command
        let value_provided = if !parts.is_empty() {
            Some(parts.join(" "))
        } else {
            None
        };

        // Try to get default value from command definition
        let value_default = self
            .commands
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.name == command.name)
            .and_then(|cmd| cmd.value.clone());

        // Use provided value if non-empty, otherwise use default
        match value_provided {
            Some(value) if !value.trim().is_empty() => Some(value),
            _ => value_default,
        }
    }

    pub fn parse(&self, input: &str) -> anyhow::Result<AppCommand> {
        // Shell commands (start with !) bypass Clap entirely.
        if input.trim().starts_with('!') {
            return Ok(AppCommand::Shell(
                input
                    .strip_prefix('!')
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            ));
        }

        let trimmed = input.trim();
        let mut tokens = trimmed.split_ascii_whitespace();
        let first = tokens.next().unwrap_or("");

        // Non-command input — pass straight through as a message.
        let is_command = first.starts_with('/') || first.starts_with(':');
        if !is_command {
            return Ok(AppCommand::Message(input.to_string()));
        }

        // Strip the sentinel character so Clap only sees the bare command name.
        let bare = first
            .strip_prefix('/')
            .or_else(|| first.strip_prefix(':'))
            .unwrap_or(first);
        let command_prefix = first
            .chars()
            .next()
            .filter(|c| *c == '/' || *c == ':')
            .unwrap_or(':');
        let rest: Vec<&str> = tokens.collect();

        // Build argv: [bare_command, arg1, arg2, …]
        let argv: Vec<&str> = std::iter::once(bare).chain(rest.iter().copied()).collect();
        let parameters: Vec<String> = rest.iter().map(|s| s.to_string()).collect();

        match ClapCmd::try_parse_from(&argv) {
            Ok(mut cmd) => {
                // Post-process variants that need Vec<String> → concrete type fixup
                match &mut cmd.sub {
                    AppCommand::Commit { args, max_diff_size } => {
                        *max_diff_size = args.iter().find_map(|p| p.parse::<usize>().ok());
                    }
                    AppCommand::Rename { name } => {
                        let n = name.join(" ");
                        let n = n.trim().to_string();
                        if n.is_empty() {
                            return Err(anyhow::anyhow!(
                                "Usage: :rename <name>. Please provide a name for the conversation."
                            ));
                        }
                    }
                    _ => {}
                }
                Ok(cmd.sub)
            }
            Err(clap_err) => {
                // Clap failed — check whether this is an agent command or a
                // registered custom workflow command before surfacing the error.
                let command_name = bare;

                // Give a domain-specific error for rename with no name argument.
                if (command_name == "rename" || command_name == "rn") && rest.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Usage: :rename <name>. Please provide a name for the conversation."
                    ));
                }

                // Check if it's an agent command pattern (agent-*)
                if command_name.starts_with("agent-") {
                    if let Some(found_command) = self.find(command_name)
                        && let Some(agent_id) = &found_command.value
                    {
                        return Ok(AppCommand::AgentSwitch(agent_id.clone()));
                    }
                    return Err(anyhow::anyhow!(
                        "/{command_name} is not a valid agent command"
                    ));
                }

                // Handle custom workflow commands
                if let Some(command) = self.find(command_name) {
                    let rest_parts: Vec<&str> = rest.to_vec();
                    let template = Template::new(
                        self.extract_command_value(&command, &rest_parts)
                            .unwrap_or_default(),
                    );
                    return Ok(AppCommand::Custom(UserCommand::new(
                        command.name.clone(),
                        template,
                        parameters,
                    )));
                }

                // Surface user-friendly errors for unknown commands.
                if clap_err.kind() == ErrorKind::InvalidSubcommand {
                    return Err(anyhow::anyhow!(
                        "Unknown command '{command_prefix}{command_name}'. Run '{command_prefix}help' to list available commands."
                    ));
                }

                // Surface a clean error from Clap (strips ANSI + internal parser name).
                let rendered = clap_err.render().to_string();
                let cleaned = rendered.replace("forge_cmd", "forge");
                Err(anyhow::anyhow!("{}", cleaned.trim()))
            }
        }
    }
}

/// Represents user input types in the chat application.
///
/// This enum encapsulates all forms of input including:
/// - System commands (starting with '/')
/// - Regular chat messages
/// - File content
#[derive(Debug, Clone, PartialEq, Eq, EnumProperty, EnumIter, Subcommand)]
pub enum AppCommand {
    /// Display the effective resolved configuration.
    /// This can be triggered with the '/config' command (aliases: env, e).
    #[strum(props(usage = "Display effective resolved configuration"))]
    #[command(aliases = ["env", "e"])]
    Config,

    /// Set the global model via interactive selection.
    /// This can be triggered with the '/config-model' command (alias: cm).
    #[strum(props(usage = "Set the global model [alias: cm]"))]
    #[command(name = "config-model", alias = "cm")]
    ConfigModel,

    /// Reset session overrides to global config.
    /// This can be triggered with the '/config-reload' command (aliases: cr,
    /// model-reset, mr).
    #[strum(props(usage = "Reset session overrides to global config [alias: cr]"))]
    #[command(name = "config-reload", aliases = ["cr", "model-reset", "mr"])]
    ConfigReload,

    /// Set the reasoning effort level.
    /// This can be triggered with the '/reasoning-effort' command (alias: re).
    #[strum(props(usage = "Set reasoning effort for current session [alias: re]"))]
    #[command(name = "reasoning-effort", alias = "re")]
    ReasoningEffort,

    /// Set the reasoning effort level in global config.
    /// This can be triggered with the '/config-reasoning-effort' command
    /// (alias: cre).
    #[strum(props(usage = "Set reasoning effort in global config [alias: cre]"))]
    #[command(name = "config-reasoning-effort", alias = "cre")]
    ConfigReasoningEffort,

    /// Set the model used for commit message generation.
    /// This can be triggered with the '/config-commit-model' command (alias:
    /// ccm).
    #[strum(props(usage = "Set the model used for commit message generation [alias: ccm]"))]
    #[command(name = "config-commit-model", alias = "ccm")]
    ConfigCommitModel,

    /// Set the model used for command suggestion generation.
    /// This can be triggered with the '/config-suggest-model' command (alias:
    /// csm).
    #[strum(props(usage = "Set the model used for suggest generation [alias: csm]"))]
    #[command(name = "config-suggest-model", alias = "csm")]
    ConfigSuggestModel,

    /// Open the global config file in an editor.
    /// This can be triggered with the '/config-edit' command (alias: ce).
    #[strum(props(usage = "Open global config file in an editor [alias: ce]"))]
    #[command(name = "config-edit", alias = "ce")]
    ConfigEdit,

    /// List all available skills.
    /// This can be triggered with the '/skill' command.
    #[strum(props(usage = "List all available skills"))]
    Skill,

    /// Open an external editor to write a prompt.
    /// This can be triggered with the '/edit' command (alias: ed).
    #[strum(props(usage = "Open external editor to write a prompt [alias: ed]"))]
    #[command(alias = "ed")]
    Edit {
        /// Initial content for the editor (optional)
        #[arg(trailing_var_arg = true, num_args = 0..)]
        content: Vec<String>,
    },

    /// Preview the AI-generated commit message without committing.
    /// This can be triggered with the '/commit-preview' command.
    #[strum(props(usage = "Preview AI-generated commit message"))]
    #[command(name = "commit-preview")]
    CommitPreview,

    /// Generate a shell command from a natural language description.
    /// This can be triggered with the '/suggest' command (alias: s).
    #[strum(props(usage = "Generate shell command from natural language [alias: s]"))]
    #[command(alias = "s")]
    Suggest {
        /// Natural language description of the shell command
        #[arg(trailing_var_arg = true, num_args = 0.., allow_hyphen_values = true)]
        description: Vec<String>,
    },

    /// Clone the current or a selected conversation.
    /// This can be triggered with the '/clone' command.
    #[strum(props(usage = "Clone current or selected conversation"))]
    Clone {
        /// Conversation ID to clone (optional — prompts interactively if
        /// absent)
        id: Option<String>,
    },

    /// Rename any conversation interactively.
    /// This can be triggered with the '/conversation-rename' command.
    #[strum(props(usage = "Rename a conversation interactively"))]
    #[command(name = "conversation-rename")]
    ConversationRename {
        /// New name for the conversation (optional — prompts interactively if
        /// absent)
        #[arg(trailing_var_arg = true, num_args = 0..)]
        name: Vec<String>,
    },

    /// Copy the last AI response to the clipboard.
    /// This can be triggered with the '/copy' command.
    #[strum(props(usage = "Copy last AI response to clipboard"))]
    Copy,

    /// Sync the current workspace for semantic search.
    /// This can be triggered with the '/workspace-sync' command (alias: sync).
    #[strum(props(usage = "Sync current workspace for semantic search [alias: sync]"))]
    #[command(name = "workspace-sync", alias = "sync")]
    WorkspaceSync,

    /// Show sync status of all workspace files.
    /// This can be triggered with the '/workspace-status' command.
    #[strum(props(usage = "Show sync status of all workspace files"))]
    #[command(name = "workspace-status", alias = "sync-status")]
    WorkspaceStatus,

    /// Show workspace information with sync details.
    /// This can be triggered with the '/workspace-info' command.
    #[strum(props(usage = "Show workspace information with sync details"))]
    #[command(name = "workspace-info", alias = "sync-info")]
    WorkspaceInfo,

    /// Initialize a new workspace without syncing files.
    /// This can be triggered with the '/workspace-init' command.
    #[strum(props(usage = "Initialize a new workspace without syncing files"))]
    #[command(name = "workspace-init", alias = "sync-init")]
    WorkspaceInit,

    /// Compact the conversation context. This can be triggered with the
    /// '/compact' command.
    #[strum(props(usage = "Compact the conversation context"))]
    Compact,

    /// Start a new conversation while preserving history.
    /// This can be triggered with the '/new' command.
    #[strum(props(usage = "Start a new conversation"))]
    New,

    /// A regular text message from the user to be processed by the chat system.
    /// Any input that doesn't start with '/' is treated as a message.
    #[strum(props(usage = "Send a regular message"))]
    #[command(skip)]
    Message(String),

    /// Display system environment information.
    /// This can be triggered with the '/info' command.
    #[strum(props(usage = "Display system information"))]
    Info,

    /// Display usage information (tokens & requests).
    #[strum(props(usage = "Shows usage information (tokens & requests)"))]
    Usage,

    /// Exit the application without any further action.
    #[strum(props(usage = "Exit the application"))]
    Exit,

    /// Updates the forge version
    #[strum(props(usage = "Updates to the latest compatible version of forge"))]
    Update,

    /// Switch to "forge" agent.
    /// This can be triggered with the '/act' command (alias: forge).
    #[strum(props(usage = "Enable implementation mode with code changes"))]
    #[command(name = "act", alias = "forge")]
    Forge,

    /// Switch to "muse" agent.
    /// This can be triggered with the '/plan' command (alias: muse).
    #[strum(props(usage = "Enable planning mode without code changes"))]
    #[command(name = "plan", alias = "muse")]
    Muse,

    /// Switch to "sage" agent.
    /// This can be triggered with the '/sage' command.
    #[strum(props(
        usage = "Enable research mode for systematic codebase exploration and analysis"
    ))]
    Sage,

    /// Switch to "help" mode.
    /// This can be triggered with the '/help' command.
    #[strum(props(usage = "Enable help mode for tool questions"))]
    #[command(name = "help")]
    Help,

    /// Dumps the current conversation into a json file or html file
    #[strum(props(usage = "Save conversation as JSON or HTML (use /dump --html for HTML format)"))]
    Dump {
        /// Output as HTML instead of JSON
        #[arg(long)]
        html: bool,
    },

    /// Switch or select the active model
    /// This can be triggered with the '/model' command.
    #[strum(props(usage = "Switch to a different model"))]
    #[command(alias = "m")]
    Model,

    /// List all available tools with their descriptions and schema
    /// This can be triggered with the '/tools' command.
    #[strum(props(usage = "List all available tools with their descriptions and schema"))]
    #[command(alias = "t")]
    Tools,

    /// Handles custom command defined in workflow file.
    #[command(skip)]
    Custom(UserCommand),

    /// Executes a native shell command.
    /// This can be triggered with commands starting with '!' character.
    #[strum(props(usage = "Execute a native shell command"))]
    #[command(skip)]
    Shell(String),

    /// Allows user to switch the operating agent.
    #[strum(props(usage = "Switch to an agent interactively"))]
    #[command(alias = "a")]
    Agent,

    /// Allows you to configure provider
    #[strum(props(usage = "Allows you to configure provider"))]
    #[command(name = "provider", aliases = ["login", "provider-login"])]
    Login,

    /// Logs out from the configured provider
    #[strum(props(usage = "Logout from configured provider"))]
    Logout,

    /// Retry without modifying model context
    #[strum(props(usage = "Retry the last command"))]
    #[command(alias = "r")]
    Retry,

    /// List all conversations for the active workspace
    #[strum(props(usage = "List all conversations for the active workspace"))]
    #[command(name = "conversation", aliases = ["conversations", "c"])]
    Conversations {
        /// Conversation ID to switch to directly (optional — shows interactive
        /// picker if absent)
        id: Option<String>,
    },

    /// Show nested conversations spawned by the current conversation
    #[strum(props(
        usage = "Show nested conversations spawned by the current conversation [alias: ct]"
    ))]
    #[command(name = "conversation-tree", alias = "ct")]
    ConversationTree,

    /// Delete a conversation permanently
    #[strum(props(usage = "Delete a conversation permanently"))]
    #[command(skip)]
    Delete,

    /// Rename the current conversation
    #[strum(props(usage = "Rename the current conversation. Usage: :rename <name>"))]
    #[command(alias = "rn")]
    Rename {
        /// New name for the conversation
        #[arg(trailing_var_arg = true, required = true)]
        name: Vec<String>,
    },

    /// Switch directly to a specific agent by ID
    #[strum(props(usage = "Switch directly to a specific agent"))]
    #[command(skip)]
    AgentSwitch(String),

    /// Generate and optionally commit changes with AI-generated message
    ///
    /// Examples:
    /// - `:commit` - Generate message and commit
    /// - `:commit 5000` - Commit with max diff of 5000 bytes
    #[strum(props(
        usage = "Generate AI commit message and commit changes. Format: :commit <max-diff|preview>"
    ))]
    Commit {
        /// Optional arguments (numeric value sets max diff size in bytes)
        #[arg(trailing_var_arg = true, num_args = 0..)]
        args: Vec<String>,
        /// Parsed max diff size (set by parse() from args)
        #[clap(skip)]
        max_diff_size: Option<usize>,
    },

    /// Index the current workspace for semantic code search
    #[strum(props(usage = "Index the current workspace for semantic search"))]
    Index,
}

impl AppCommand {
    pub fn name(&self) -> &str {
        match self {
            AppCommand::Compact => "compact",
            AppCommand::New => "new",
            AppCommand::Message(_) => "message",
            AppCommand::Update => "update",
            AppCommand::Info => "info",
            AppCommand::Usage => "usage",
            AppCommand::Exit => "exit",
            AppCommand::Forge => "forge",
            AppCommand::Muse => "muse",
            AppCommand::Sage => "sage",
            AppCommand::Help => "help",
            AppCommand::Commit { .. } => "commit",
            AppCommand::Dump { .. } => "dump",
            AppCommand::Model => "model",
            AppCommand::Tools => "tools",
            AppCommand::Custom(event) => &event.name,
            AppCommand::Shell(_) => "!shell",
            AppCommand::Agent => "agent",
            AppCommand::Login => "login",
            AppCommand::Logout => "logout",
            AppCommand::Retry => "retry",
            AppCommand::Conversations { .. } => "conversation",
            AppCommand::ConversationTree => "conversation-tree",
            AppCommand::Delete => "delete",
            AppCommand::Rename { .. } => "rename",
            AppCommand::AgentSwitch(agent_id) => agent_id,
            AppCommand::Index => "index",
            AppCommand::Config => "config",
            AppCommand::ConfigModel => "config-model",
            AppCommand::ConfigReload => "config-reload",
            AppCommand::ReasoningEffort => "reasoning-effort",
            AppCommand::ConfigReasoningEffort => "config-reasoning-effort",
            AppCommand::ConfigCommitModel => "config-commit-model",
            AppCommand::ConfigSuggestModel => "config-suggest-model",
            AppCommand::ConfigEdit => "config-edit",
            AppCommand::Skill => "skill",
            AppCommand::Edit { .. } => "edit",
            AppCommand::CommitPreview => "commit-preview",
            AppCommand::Suggest { .. } => "suggest",
            AppCommand::Clone { .. } => "clone",
            AppCommand::ConversationRename { .. } => "conversation-rename",
            AppCommand::Copy => "copy",
            AppCommand::WorkspaceSync => "workspace-sync",
            AppCommand::WorkspaceStatus => "workspace-status",
            AppCommand::WorkspaceInfo => "workspace-info",
            AppCommand::WorkspaceInit => "workspace-init",
        }
    }

    /// Returns the usage description for the command.
    pub fn usage(&self) -> &str {
        self.get_str("usage").unwrap()
    }

    /// Returns true for internal/meta variants that should not appear in the
    /// public `forge list commands` output or the REPL help listing.
    pub fn is_internal(&self) -> bool {
        matches!(
            self,
            AppCommand::Message(_)
                | AppCommand::Custom(_)
                | AppCommand::Shell(_)
                | AppCommand::AgentSwitch(_)
                | AppCommand::Rename { .. }
        )
    }

    /// Returns true for variants that are pure agent-switch shorthands whose
    /// canonical name matches a built-in agent (forge, muse, sage).  These
    /// commands are already emitted as AGENT rows by the agent-info loop in
    /// `on_show_commands`, so they must be excluded from the COMMAND loop to
    /// avoid duplicate entries in `list commands --porcelain`.
    pub fn is_agent_switch(&self) -> bool {
        matches!(
            self,
            AppCommand::Forge | AppCommand::Muse | AppCommand::Sage
        )
    }
}

#[cfg(test)]
#[path = "../test/command_test.rs"]
mod tests;

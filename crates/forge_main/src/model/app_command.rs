//! Definitions of all built-in application commands.

use clap::Subcommand;
use forge_domain::UserCommand;
use strum::EnumProperty;
use strum_macros::{EnumIter, EnumProperty};

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
    /// This can be triggered with the '/reasoning-effort' command.
    #[strum(props(usage = "Set reasoning effort for current session"))]
    #[command(name = "effort")]
    Effort,

    /// Set the reasoning effort level in global config.
    /// This can be triggered with the '/config-reasoning-effort' command
    /// (alias: cre).
    #[strum(props(usage = "Set reasoning effort in global config [alias: cre]"))]
    #[command(name = "config-reasoning-effort", alias = "cre")]
    ConfigReasoningEffort,

    /// Set the model used for commit message generation.
    /// This can be triggered with the '/commit-model' command.
    #[strum(props(usage = "Set the model used for commit message generation [alias: ccm]"))]
    #[command(name = "commit-model")]
    CommitModel,

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
    /// ---
    /// I don't know why not .md, this legacy command should be replaced with `/save`
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
    /// Returns the canonical command name used in listings and dispatch.
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
            AppCommand::Effort => "effort",
            AppCommand::ConfigReasoningEffort => "config-reasoning-effort",
            AppCommand::CommitModel => "commit-model",
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

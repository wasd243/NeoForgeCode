//! Runtime registry of commands available in the REPL: built-in commands,
//! user-defined workflow commands, and agent-switch commands.

use std::sync::{Arc, Mutex};

use forge_api::AgentInfo;
use strum::IntoEnumIterator;

use super::AppCommand;

/// A single entry in the command registry, as shown in completion and help
/// listings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeCommand {
    pub name: String,
    pub description: String,
    pub value: Option<String>,
}

/// Result of agent command registration
#[derive(Debug, Clone)]
pub struct AgentCommandRegistrationResult {
    pub registered_count: usize,
    pub skipped_conflicts: Vec<String>,
}

/// Thread-safe registry of all commands available in the current session.
///
/// Holds the built-in commands by default and allows registering custom
/// workflow commands and agent-switch commands at runtime.
#[derive(Debug)]
pub struct ForgeCommandManager {
    pub(super) commands: Arc<Mutex<Vec<ForgeCommand>>>,
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
    pub(super) fn sanitize_agent_id(agent_id: &str) -> String {
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
    pub(super) fn is_reserved_command(name: &str) -> bool {
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
                | "effort"
                | "re"
                | "config-reasoning-effort"
                | "cre"
                | "commit-model"
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

    /// Builds the initial registry from all non-internal built-in commands.
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
    pub(super) fn find(&self, command: &str) -> Option<ForgeCommand> {
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
}

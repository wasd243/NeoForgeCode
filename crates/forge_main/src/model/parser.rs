//! Parsing of raw REPL input into [`AppCommand`] values.
//!
//! Handles the `!` shell bypass, the `/` and `:` command sentinels, Clap
//! dispatch for built-in commands, and fallback resolution of agent-switch
//! and custom workflow commands from the registry.

use clap::Parser;
use clap::error::ErrorKind;
use forge_api::Template;
use forge_domain::UserCommand;

use super::registry::ForgeCommand;
use super::{AppCommand, ForgeCommandManager};

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

impl ForgeCommandManager {
    /// Extracts the command value from the input parts
    ///
    /// # Arguments
    /// * `command` - The command for which to extract the value
    /// * `parts` - The parts of the command input after the command name
    ///
    /// # Returns
    /// * `Option<String>` - The extracted value, if any
    pub(super) fn extract_command_value(
        &self,
        command: &ForgeCommand,
        parts: &[&str],
    ) -> Option<String> {
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

    /// Parses raw user input into an [`AppCommand`].
    ///
    /// # Errors
    /// Returns an error for unknown commands, invalid arguments, or malformed
    /// agent commands.
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
        let is_command = first.starts_with('/');
        if !is_command {
            return Ok(AppCommand::Message(input.to_string()));
        }

        // Strip the sentinel character so Clap only sees the bare command name.
        let bare = first
            .strip_prefix('/')
            .unwrap_or(first);
        let command_prefix = first
            .chars()
            .next()
            .filter(|c| *c == '/')
            .unwrap(); // Why remove all `:` characters? because I don't like it.
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

//! Command model for the interactive REPL.
//!
//! Split by responsibility:
//! - [`app_command`]: the [`AppCommand`] enum defining every built-in command
//! - [`registry`]: the runtime [`ForgeCommandManager`] registry of built-in,
//!   custom workflow, and agent-switch commands
//! - [`parser`]: parsing of raw user input into [`AppCommand`] values

mod app_command;
mod parser;
mod registry;

pub use app_command::AppCommand;
pub use registry::{ForgeCommand, ForgeCommandManager};

#[cfg(test)]
#[path = "../tests/command_test.rs"]
mod tests;

use std::str::FromStr;

use colored::Colorize;
use forge_api::{API, AgentId, Conversation, ConversationId};
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, TitleFormat};
use forge_select::ForgeWidget;

use super::UI;
use crate::zsh::ZshRPrompt;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    /// Generate ZSH plugin script
    pub(super) async fn on_zsh_plugin(&self) -> anyhow::Result<()> {
        let plugin = crate::zsh::generate_zsh_plugin()?;
        println!("{plugin}");
        Ok(())
    }

    /// Generate ZSH theme
    pub(super) async fn on_zsh_theme(&self) -> anyhow::Result<()> {
        let theme = crate::zsh::generate_zsh_theme()?;
        println!("{theme}");
        Ok(())
    }

    /// Run ZSH environment diagnostics
    pub(super) async fn on_zsh_doctor(&mut self) -> anyhow::Result<()> {
        // Stop spinner before streaming output to avoid interference
        self.spinner.stop(None)?;

        // Stream the diagnostic output in real-time
        crate::zsh::run_zsh_doctor()?;

        Ok(())
    }

    /// Show ZSH keyboard shortcuts
    pub(super) async fn on_zsh_keyboard(&mut self) -> anyhow::Result<()> {
        // Stop spinner before streaming output to avoid interference
        self.spinner.stop(None)?;

        // Stream the keyboard shortcuts output in real-time
        crate::zsh::run_zsh_keyboard()?;

        Ok(())
    }

    /// Install the Forge VS Code extension
    pub(super) async fn on_vscode_extension_install(&mut self) -> anyhow::Result<()> {
        self.spinner
            .start(Some("Installing Forge VS Code extension"))?;

        match crate::vscode::install_extension() {
            Ok(true) => {
                self.spinner.stop(None)?;
                self.writeln_title(TitleFormat::info(
                    "Forge VS Code extension installed successfully",
                ))?;
            }
            Ok(false) => {
                self.spinner.stop(None)?;
                self.writeln_title(TitleFormat::error(
                    "Failed to install Forge VS Code extension.",
                ))?;
            }
            Err(e) => {
                self.spinner.stop(None)?;
                self.writeln_title(TitleFormat::error(format!(
                    "Failed to install Forge VS Code extension: {e}"
                )))?;
            }
        }

        Ok(())
    }

    /// Setup ZSH integration by updating .zshrc
    pub(super) async fn on_zsh_setup(&mut self) -> anyhow::Result<()> {
        // Check nerd font support
        println!();
        println!(
            "{} {} {}",
            "󱙺".bold(),
            "FORGE 33.0k".bold(),
            " tonic-1.0".cyan()
        );

        let can_see_nerd_fonts =
            ForgeWidget::confirm("Can you see all the icons clearly without any overlap?")
                .with_default(true)
                .prompt()?;

        let disable_nerd_font = match can_see_nerd_fonts {
            Some(true) => {
                println!();
                false
            }
            Some(false) => {
                println!();
                println!("   {} Nerd Fonts will be disabled", "⚠".yellow());
                println!();
                println!("   You can enable them later by:");
                println!(
                    "   1. Installing a Nerd Font from: {}",
                    "https://www.nerdfonts.com/".dimmed()
                );
                println!("   2. Configuring your terminal to use a Nerd Font");
                println!(
                    "   3. Removing {} from your ~/.zshrc",
                    "NERD_FONT=0".dimmed()
                );
                println!();
                true
            }
            None => {
                // User interrupted, default to not disabling
                println!();
                false
            }
        };

        // Ask about editor preference
        let editor_options = vec![
            "Use system default ($EDITOR)",
            "VS Code (code --wait)",
            "Vim",
            "Neovim (nvim)",
            "Nano",
            "Emacs",
            "Sublime Text (subl --wait)",
            "Skip - I'll configure it later",
        ];

        let selected_editor = ForgeWidget::select(
            "Which editor would you like to use for editing prompts?",
            editor_options,
        )
        .prompt()?;

        let forge_editor = match selected_editor {
            Some("Use system default ($EDITOR)") => None,
            Some("VS Code (code --wait)") => Some("code --wait"),
            Some("Vim") => Some("vim"),
            Some("Neovim (nvim)") => Some("nvim"),
            Some("Nano") => Some("nano"),
            Some("Emacs") => Some("emacs"),
            Some("Sublime Text (subl --wait)") => Some("subl --wait"),
            Some("Skip - I'll configure it later") => None,
            _ => None,
        };

        // Setup ZSH integration with nerd font and editor configuration
        self.spinner.start(Some("Configuring ZSH"))?;
        let result = crate::zsh::setup_zsh_integration(disable_nerd_font, forge_editor)?;
        self.spinner.stop(None)?;

        // Log backup creation if one was made
        if let Some(backup_path) = result.backup_path {
            self.writeln_title(TitleFormat::debug(format!(
                "backup created at {}",
                backup_path.display()
            )))?;
        }

        self.writeln_title(TitleFormat::info(result.message))?;

        self.writeln_title(TitleFormat::debug("running forge zsh doctor"))?;
        println!();
        let doctor_result = self.on_zsh_doctor().await;

        if doctor_result.is_ok() {
            self.writeln_title(TitleFormat::action(
                "run `exec zsh` now (or open a new terminal window) to load the updated shell config",
            ))?;
            self.writeln_title(TitleFormat::action(
                "run `: Hi` after restarting your shell to confirm everything works",
            ))?;
        }

        doctor_result
    }

    /// Renders the ZSH right-prompt (rprompt) string from the current session
    /// state propagated via environment variables by the shell plugin.
    pub(super) async fn handle_zsh_rprompt_command(&mut self) -> Option<String> {
        let cid = std::env::var("_FORGE_CONVERSATION_ID")
            .ok()
            .filter(|text| !text.trim().is_empty())
            .and_then(|str| ConversationId::from_str(str.as_str()).ok());

        let agent_id = std::env::var("_FORGE_ACTIVE_AGENT")
            .ok()
            .filter(|text| !text.trim().is_empty())
            .map(AgentId::new);

        // Make IO calls in parallel
        let (model_id, conversation, reasoning_effort) = tokio::join!(
            self.get_agent_model(agent_id.clone()),
            async {
                if let Some(cid) = cid {
                    self.api.conversation(&cid).await.ok().flatten()
                } else {
                    None
                }
            },
            async { self.api.get_reasoning_effort().await.ok().flatten() }
        );

        // Calculate total cost including related conversations
        let cost = if let Some(ref conv) = conversation {
            let related_conversations = self.fetch_related_conversations(conv).await;
            let all_conversations: Vec<_> = std::iter::once(conv)
                .chain(related_conversations.iter())
                .cloned()
                .collect();
            Conversation::total_cost(&all_conversations)
        } else {
            None
        };

        // Check if nerd fonts should be used (NERD_FONT or USE_NERD_FONT set to "1")
        let use_nerd_font = std::env::var("NERD_FONT")
            .or_else(|_| std::env::var("USE_NERD_FONT"))
            .map(|val| val == "1")
            .unwrap_or(true); // Default to true

        // Read terminal width from COLUMNS (propagated by the zsh shell plugin)
        // so the rprompt can pick a compact or full-length reasoning effort
        // label. Missing or unparseable values fall back to the full-length
        // form in the renderer.
        let terminal_width = std::env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());

        let rprompt = ZshRPrompt::from_config(&self.config)
            .agent(agent_id)
            .model(model_id)
            .token_count(conversation.and_then(|conversation| conversation.token_count()))
            .cost(cost)
            .reasoning_effort(reasoning_effort)
            .terminal_width(terminal_width)
            .use_nerd_font(use_nerd_font);

        Some(rprompt.to_string())
    }
}

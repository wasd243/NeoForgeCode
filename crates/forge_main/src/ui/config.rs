use anyhow::Result;
use colored::Colorize;
use forge_api::{API, ConfigOperation};
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, TitleFormat};

use super::UI;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    /// Lists current configuration values
    pub(super) async fn on_show_config(&mut self, porcelain: bool) -> anyhow::Result<()> {
        // Get the effective resolved config
        let config = &self.config;

        // Serialize to TOML pretty format
        let config_toml = toml_edit::ser::to_string_pretty(config)
            .map_err(|e| anyhow::anyhow!("Failed to serialize config: {}", e))?;

        if porcelain {
            // For porcelain mode, output raw TOML without highlighting
            self.writeln(config_toml)?;
        } else {
            // For human-readable mode, add a title and syntax-highlight the TOML
            self.writeln("\nCONFIGURATION\n".bold().dimmed())?;
            let highlighted =
                forge_display::SyntaxHighlighter::default().highlight(&config_toml, "toml");
            self.writeln(highlighted)?;
        }

        Ok(())
    }

    /// Selects and sets the reasoning effort level interactively.
    ///
    /// # Arguments
    /// * `global` - If true, persists the change to the global config file. If
    ///   false, applies to the session (REPL has no separate session scope, so
    ///   this always writes to the config).
    pub(super) async fn on_reasoning_effort_selection(&mut self, global: bool) -> anyhow::Result<()> {
        use std::str::FromStr;

        let prompt = if global {
            "Config Reasoning Effort"
        } else {
            "Reasoning Effort"
        };

        let selected = self.select_reasoning_effort(prompt, None).await?;

        if let Some(effort_str) = selected {
            let effort = forge_domain::Effort::from_str(&effort_str)
                .map_err(|_| anyhow::anyhow!("Invalid effort level: {effort_str}"))?;
            self.api
                .update_config(vec![ConfigOperation::SetReasoningEffort(effort.clone())])
                .await?;
            self.writeln_title(
                TitleFormat::action(effort_str).sub_title("is now the reasoning effort"),
            )?;
        }

        Ok(())
    }

    /// Selects and sets the commit model via interactive model picker.
    pub(super) async fn on_config_commit_model(&mut self) -> anyhow::Result<()> {
        let selection = self.select_model(None, None).await?;
        if let Some((model, provider_id)) = selection {
            let commit_config = forge_domain::ModelConfig::new(provider_id.clone(), model.clone());
            self.api
                .update_config(vec![ConfigOperation::SetCommitConfig(Some(commit_config))])
                .await?;
            self.writeln_title(TitleFormat::action(model.as_str()).sub_title(format!(
                "is now the commit model for provider '{provider_id}'"
            )))?;
        }
        Ok(())
    }

    /// Selects and sets the suggest model via interactive model picker.
    pub(super) async fn on_config_suggest_model(&mut self) -> anyhow::Result<()> {
        let selection = self.select_model(None, None).await?;
        if let Some((model, provider_id)) = selection {
            let suggest_config = forge_domain::ModelConfig::new(provider_id.clone(), model.clone());
            self.api
                .update_config(vec![ConfigOperation::SetSuggestConfig(suggest_config)])
                .await?;
            self.writeln_title(TitleFormat::action(model.as_str()).sub_title(format!(
                "is now the suggest model for provider '{provider_id}'"
            )))?;
        }
        Ok(())
    }

    /// Opens the global config file in the system editor.
    pub(super) async fn on_config_edit(&mut self) -> anyhow::Result<()> {
        let config_path = forge_config::ConfigReader::config_path();

        // Ensure parent directory exists
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Create config file if it does not exist
        if !config_path.exists() {
            std::fs::File::create(&config_path)?;
        }

        let editor = std::env::var("FORGE_EDITOR")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "nano".to_string());
        let editor_binary = editor
            .split_whitespace()
            .next()
            .unwrap_or("nano")
            .to_string();

        let status = std::process::Command::new(&editor_binary)
            .arg(&config_path)
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

        self.writeln_title(TitleFormat::info(format!(
            "Config saved: {}",
            config_path.display()
        )))?;

        Ok(())
    }

    /// Handle config command
    pub(super) async fn handle_config_command(
        &mut self,
        command: crate::cli::ConfigCommand,
        porcelain: bool,
    ) -> Result<()> {
        match command {
            crate::cli::ConfigCommand::Set(args) => self.handle_config_set(args).await?,
            crate::cli::ConfigCommand::Get(args) => self.handle_config_get(args).await?,
            crate::cli::ConfigCommand::List => {
                self.on_show_config(porcelain).await?;
            }
            crate::cli::ConfigCommand::Path => {
                let path = forge_config::ConfigReader::config_path();
                self.writeln(path.display().to_string())?;
            }
            crate::cli::ConfigCommand::Migrate => {
                self.handle_config_migrate()?;
            }
        }
        Ok(())
    }

    /// Rename `~/forge` to `~/.forge`.
    ///
    /// Errors if the legacy directory does not exist, if the new directory
    /// already exists, or if the rename fails.
    pub(super) fn handle_config_migrate(&mut self) -> Result<()> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        let legacy = home.join("forge");
        let new = home.join(".forge");

        if !legacy.exists() {
            anyhow::bail!(
                "Legacy directory {} does not exist — nothing to migrate",
                legacy.display()
            );
        }

        if new.exists() {
            anyhow::bail!(
                "Target directory {} already exists — remove it first or migrate manually",
                new.display()
            );
        }

        std::fs::rename(&legacy, &new).map_err(|e| {
            anyhow::anyhow!(
                "Failed to rename {} to {}: {}",
                legacy.display(),
                new.display(),
                e
            )
        })?;

        self.writeln_title(TitleFormat::info("Migration Completed").sub_title(format!(
            "{} → {}",
            legacy.display(),
            new.display()
        )))?;

        Ok(())
    }

    /// Handle config set command
    pub(super) async fn handle_config_set(&mut self, args: crate::cli::ConfigSetArgs) -> Result<()> {
        use crate::cli::ConfigSetField;

        match args.field {
            ConfigSetField::Model { provider, model } => {
                let provider = self.api.get_provider(&provider).await?;
                self.activate_provider_with_model(provider, Some(model))
                    .await?;
            }
            ConfigSetField::Commit { provider, model } => {
                // Validate provider exists and model belongs to that specific provider
                let validated_model = self.validate_model(model.as_str(), Some(&provider)).await?;
                let commit_config =
                    forge_domain::ModelConfig::new(provider.clone(), validated_model.clone());
                self.api
                    .update_config(vec![ConfigOperation::SetCommitConfig(Some(commit_config))])
                    .await?;
                self.writeln_title(
                    TitleFormat::action(validated_model.as_str())
                        .sub_title(format!("is now the commit model for provider '{provider}'")),
                )?;
            }
            ConfigSetField::Suggest { provider, model } => {
                // Validate provider exists and model belongs to that specific provider
                let validated_model = self.validate_model(model.as_str(), Some(&provider)).await?;
                let suggest_config =
                    forge_domain::ModelConfig::new(provider.clone(), validated_model.clone());
                self.api
                    .update_config(vec![ConfigOperation::SetSuggestConfig(suggest_config)])
                    .await?;
                self.writeln_title(TitleFormat::action(validated_model.as_str()).sub_title(
                    format!("is now the suggest model for provider '{provider}'"),
                ))?;
            }
            ConfigSetField::ReasoningEffort { effort } => {
                self.api
                    .update_config(vec![ConfigOperation::SetReasoningEffort(effort.clone())])
                    .await?;
                self.writeln_title(
                    TitleFormat::action(effort.to_string())
                        .sub_title("is now the reasoning effort"),
                )?;
            }
        }

        Ok(())
    }

    /// Handle config get command
    pub(super) async fn handle_config_get(&mut self, args: crate::cli::ConfigGetArgs) -> Result<()> {
        use crate::cli::ConfigGetField;

        match args.field {
            ConfigGetField::Model => {
                let model = self
                    .api
                    .get_session_config()
                    .await
                    .map(|c| c.model.as_str().to_string());
                match model {
                    Some(v) => self.writeln(v.to_string())?,
                    None => self.writeln("Model: Not set")?,
                }
            }
            ConfigGetField::Provider => {
                let provider = self
                    .api
                    .get_session_config()
                    .await
                    .map(|c| c.provider.to_string());
                match provider {
                    Some(v) => self.writeln(v.to_string())?,
                    None => self.writeln("Provider: Not set")?,
                }
            }
            ConfigGetField::Commit => {
                let commit_config = self.api.get_commit_config().await?;
                match commit_config {
                    Some(config) => {
                        self.writeln(config.provider.as_ref())?;
                        self.writeln(config.model.as_str().to_string())?;
                    }
                    None => self.writeln("Commit: Not set")?,
                }
            }
            ConfigGetField::Suggest => {
                let suggest_config = self.api.get_suggest_config().await?;
                match suggest_config {
                    Some(config) => {
                        self.writeln(config.provider.as_ref())?;
                        self.writeln(config.model.as_str().to_string())?;
                    }
                    None => self.writeln("Suggest: Not set")?,
                }
            }
            ConfigGetField::ReasoningEffort => {
                let effort = self.api.get_reasoning_effort().await?;
                match effort {
                    Some(e) => self.writeln(e.to_string())?,
                    None => self.writeln("ReasoningEffort: Not set")?,
                }
            }
        }

        Ok(())
    }
}

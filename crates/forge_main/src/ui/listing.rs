use anyhow::{Context, Result};
use convert_case::{Case, Casing};
use forge_api::{API, AgentId, ConversationId};
use forge_app::ToolResolver;
use forge_app::utils::{format_display_path, truncate_key};
use forge_config::ForgeConfig;
use forge_domain::ConsoleWriter;
use forge_walker::Walker;
use strum::IntoEnumIterator;

use super::UI;
use crate::display_constants::{CommandType, headers, markers, status};
use crate::info::Info;
use crate::model::{AppCommand, ForgeCommandManager};
use crate::porcelain::Porcelain;
use crate::tools_display::format_tools;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    /// Builds an Info structure for agents with their details
    pub(super) async fn build_agents_info(&self, custom: bool) -> anyhow::Result<Info> {
        let mut agents = self.api.get_agents().await?;
        // Sort agents alphabetically by ID
        agents.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

        // Filter agents based on custom flag
        if custom {
            agents.retain(|agent| agent.path.is_some());
        }

        let mut info = Info::new();

        for agent in agents.iter() {
            let id = agent.id.as_str().to_string();
            let title = agent
                .title
                .as_deref()
                .map(|title| title.lines().collect::<Vec<_>>().join(" "));

            // Get provider and model for this agent
            let provider_name = match self.get_provider(Some(agent.id.clone())).await {
                Ok(p) => p.id.to_string(),
                Err(e) => format!("Error: [{}]", e),
            };

            let model_name = agent.model.as_str().to_string();

            let reasoning = if agent
                .reasoning
                .as_ref()
                .and_then(|a| a.enabled)
                .unwrap_or_default()
            {
                status::YES
            } else {
                status::NO
            };

            let location = agent
                .path
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| markers::BUILT_IN.to_string());

            info = info
                .add_title(id.to_case(Case::UpperSnake))
                .add_key_value("Id", id)
                .add_key_value("Title", title)
                .add_key_value("Location", location)
                .add_key_value("Provider", provider_name)
                .add_key_value("Model", model_name)
                .add_key_value("Reasoning Enabled", reasoning);
        }

        Ok(info)
    }

    pub(super) async fn on_show_agents(&mut self, porcelain: bool, custom: bool) -> anyhow::Result<()> {
        let agents = self.api.get_agent_infos().await?;

        if agents.is_empty() {
            return Ok(());
        }

        let info = self.build_agents_info(custom).await?;

        if porcelain {
            let porcelain = Porcelain::from(&info)
                .drop_col(0)
                .truncate(3, 60)
                .uppercase_headers();
            self.writeln(porcelain)?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Lists all the providers
    pub(super) async fn on_show_providers(
        &mut self,
        porcelain: bool,
        types: Vec<forge_domain::ProviderType>,
    ) -> anyhow::Result<()> {
        let mut providers = self.api.get_providers().await?;

        // Filter by type if specified
        if !types.is_empty() {
            providers.retain(|p| types.contains(p.provider_type()));
        }

        if providers.is_empty() {
            return Ok(());
        }

        let mut info = Info::new();

        for provider in providers.iter() {
            let id: &str = &provider.id();
            let display_name = provider.id().to_string();
            let domain = if let Some(url) = provider.url() {
                url.domain().map(|d| d.to_string()).unwrap_or_default()
            } else {
                markers::EMPTY.to_string()
            };
            let configured = provider.is_configured();
            info = info
                .add_title(id.to_case(Case::UpperSnake))
                .add_key_value("name", display_name)
                .add_key_value("id", id)
                .add_key_value("host", domain);
            if configured {
                info = info.add_key_value("logged in", status::YES);
            };
        }

        if porcelain {
            let porcelain = Porcelain::from(&info).drop_col(0).uppercase_headers();
            self.writeln(porcelain)?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Lists all the models
    pub(super) async fn on_show_models(&mut self, porcelain: bool) -> anyhow::Result<()> {
        self.spinner.start(Some("Fetching Models"))?;

        let mut all_provider_models = match self.api.get_all_provider_models().await {
            Ok(provider_models) => provider_models,
            Err(err) => {
                self.spinner.stop(None)?;
                return Err(err);
            }
        };

        if all_provider_models.is_empty() {
            return Ok(());
        }

        // Sort models and then providers
        all_provider_models
            .iter_mut()
            .for_each(|pm| pm.models.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str())));
        all_provider_models.sort_by(|a, b| a.provider_id.as_ref().cmp(b.provider_id.as_ref()));

        let mut info = Info::new();
        for pm in &all_provider_models {
            let provider_id: &str = &pm.provider_id;
            let provider_display = pm.provider_id.to_string();
            for model in &pm.models {
                let id = model.id.to_string();
                info = info
                    .add_title(&id)
                    .add_key_value("Model", model.name.as_ref().unwrap_or(&id))
                    .add_key_value("Provider", &provider_display)
                    .add_key_value("Provider Id", provider_id);

                // Add context length if available, otherwise use "unknown"
                if let Some(limit) = model.context_length {
                    let context = if limit >= 1_000_000 {
                        format!("{}M", limit / 1_000_000)
                    } else if limit >= 1000 {
                        format!("{}k", limit / 1000)
                    } else {
                        format!("{limit}")
                    };
                    info = info.add_key_value("Context Window", context);
                } else {
                    info = info.add_key_value("Context Window", markers::EMPTY)
                }

                // Add tools support indicator if explicitly supported
                if let Some(supported) = model.tools_supported {
                    info = info.add_key_value(
                        "Tool Supported",
                        if supported { status::YES } else { status::NO },
                    )
                } else {
                    info = info.add_key_value("Tools", markers::EMPTY)
                }

                // Add image modality support indicator
                let supports_image = model
                    .input_modalities
                    .contains(&forge_domain::InputModality::Image);
                info = info.add_key_value(
                    "Image",
                    if supports_image {
                        status::YES
                    } else {
                        status::NO
                    },
                );
            }
        }

        if porcelain {
            self.writeln(Porcelain::from(&info).truncate(1, 40).uppercase_headers())?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    pub(super) async fn commands_porcelain(&self) -> Result<Porcelain> {
        let custom_commands = self.api.get_commands().await?;
        let mut info = Info::new();

        for cmd in AppCommand::iter().filter(|c| !c.is_internal() && !c.is_agent_switch()) {
            info = info
                .add_title(cmd.name())
                .add_key_value("type", CommandType::Command)
                .add_key_value("description", cmd.usage());
        }

        info = info
            .add_title("ask")
            .add_key_value("type", CommandType::Agent)
            .add_key_value(
                "description",
                "Research and investigation agent [alias for: sage]",
            )
            .add_title("plan")
            .add_key_value("type", CommandType::Agent)
            .add_key_value(
                "description",
                "Planning and strategy agent [alias for: muse]",
            );

        let agent_infos = self.api.get_agent_infos().await?;
        for agent_info in agent_infos {
            let title = agent_info
                .title
                .map(|title| title.lines().collect::<Vec<_>>().join(" "));
            info = info
                .add_title(agent_info.id.to_string())
                .add_key_value("type", CommandType::Agent)
                .add_key_value("description", title);
        }

        for command in custom_commands {
            info = info
                .add_title(command.name.clone())
                .add_key_value("type", CommandType::Custom)
                .add_key_value("description", command.description.clone());
        }

        Ok(Porcelain::from(&info)
            .uppercase_headers()
            .sort_by(&[1, 0])
            .to_case(&[1], Case::UpperSnake)
            .map_col(0, |col| {
                if col.as_deref() == Some(headers::ID) {
                    Some("COMMAND".to_string())
                } else {
                    col
                }
            }))
    }

    /// Lists all the commands
    pub(super) async fn on_show_commands(&mut self, porcelain: bool) -> anyhow::Result<()> {
        let custom_commands = self.api.get_commands().await?;

        if porcelain {
            self.writeln(self.commands_porcelain().await?)?;
        } else {
            // Non-porcelain: render in the same flat format as :help in the REPL.
            let command_manager = ForgeCommandManager::default();
            command_manager.register_all(custom_commands);
            let info = Info::from(&command_manager);
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Lists only custom commands (used by `forge run`)
    pub(super) async fn on_show_custom_commands(&mut self, porcelain: bool) -> anyhow::Result<()> {
        let custom_commands = self.api.get_commands().await?;
        let mut info = Info::new();

        for command in custom_commands {
            info = info
                .add_title(command.name.clone())
                .add_key_value("description", command.description.clone());
        }

        if porcelain {
            let porcelain = Porcelain::from(&info).uppercase_headers();
            self.writeln(porcelain)?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Lists available skills
    pub(super) async fn on_show_skills(&mut self, porcelain: bool, custom: bool) -> anyhow::Result<()> {
        let skills = self.api.get_skills().await?;

        // Filter skills based on custom flag
        let skills = if custom {
            skills
                .into_iter()
                .filter(|skill| skill.path.is_some())
                .collect()
        } else {
            skills
        };

        let mut info = Info::new();
        let env = self.api.environment();

        for skill in skills {
            info = info
                .add_title(skill.name.clone().to_case(Case::Sentence).to_uppercase())
                .add_key_value("name", skill.name);

            if let Some(path) = skill.path {
                info = info.add_key_value("path", format_display_path(&path, &env.cwd));
            }

            info = info.add_key_value("description", skill.description);
        }

        if porcelain {
            let porcelain = Porcelain::from(&info)
                .drop_col(0)
                .truncate(2, 60)
                .uppercase_headers();
            self.writeln(porcelain)?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Lists files and directories in the current workspace.
    ///
    /// Uses the same `Walker::max_all()` configuration as the REPL file picker
    /// and the shell plugin (`fd --type f --type d --hidden --exclude .git`):
    /// hidden files included, respects `.gitignore`, directories suffixed with
    /// `/`.
    pub(super) async fn on_list_files(&mut self, porcelain: bool) -> anyhow::Result<()> {
        let env = self.api.environment();
        let files = Walker::max_all()
            .cwd(env.cwd.clone())
            .get()
            .await
            .context("Failed to walk workspace files")?;

        if porcelain {
            for file in files {
                self.writeln(file.path)?;
            }
        } else {
            let mut info = Info::new();
            for file in &files {
                info = info.add_key_value("path", file.path.clone());
            }
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Displays available tools for the current agent
    pub(super) async fn on_show_tools(&mut self, agent_id: AgentId, porcelain: bool) -> anyhow::Result<()> {
        self.spinner.start(Some("Loading"))?;
        let all_tools = self.api.get_tools().await?;
        let agents = self.api.get_agents().await?;
        let agent = agents.into_iter().find(|agent| agent.id == agent_id);
        let agent_tools = if let Some(agent) = agent {
            let resolver = ToolResolver::new(all_tools.clone().into());
            resolver
                .resolve(&agent)
                .into_iter()
                .map(|def| def.name.clone())
                .collect()
        } else {
            Vec::new()
        };

        let info = format_tools(&agent_tools, &all_tools);
        if porcelain {
            self.writeln(
                Porcelain::from(&info)
                    .into_long()
                    .drop_col(1)
                    .uppercase_headers(),
            )?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    pub(super) async fn on_info(
        &mut self,
        porcelain: bool,
        conversation_id: Option<ConversationId>,
    ) -> anyhow::Result<()> {
        let mut info = Info::new();

        // Fetch conversation
        let conversation = match conversation_id {
            Some(conversation_id) => self.api.conversation(&conversation_id).await.ok().flatten(),
            None => None,
        };

        // Fetch agent
        let agent = self.api.get_active_agent().await;

        // Fetch model (resolved with default model if unset)
        let model = self.get_agent_model(agent.clone()).await;

        // Fetch agent-specific provider or default provider if unset
        let agent_provider = self.get_provider(agent.clone()).await.ok();

        // Fetch default provider (could be different from the set provider)
        let default_provider = self.api.get_default_provider().await.ok();

        // Add agent information
        info = info.add_title("AGENT");
        if let Some(agent) = agent {
            info = info.add_key_value("ID", agent.as_str().to_uppercase());
        }

        // Add model information if available
        if let Some(model) = model {
            info = info.add_key_value("Model", model.as_str());
        }

        // Add provider information
        match (default_provider, agent_provider) {
            (Some(default), Some(agent_specific)) if default.id != agent_specific.id => {
                // Show both providers if they're different
                info = info.add_key_value("Agent Provider (URL)", agent_specific.url.as_str());
                if let Some(api_key) = agent_specific.api_key() {
                    info = info.add_key_value("Agent API Key", truncate_key(api_key.as_str()));
                }

                info = info.add_key_value("Default Provider (URL)", default.url.as_str());
                if let Some(api_key) = default.api_key() {
                    info = info.add_key_value("Default API Key", truncate_key(api_key.as_str()));
                }
            }
            (Some(provider), _) | (_, Some(provider)) => {
                // Show single provider (either default or agent-specific)
                info = info.add_key_value("Provider (URL)", provider.url.as_str());
                if let Some(api_key) = provider.api_key() {
                    info = info.add_key_value("API Key", truncate_key(api_key.as_str()));
                }
            }
            _ => {
                // No provider available
            }
        }

        // Add conversation information if available
        if let Some(conversation) = conversation {
            info = info.extend(Info::from(&conversation));
        } else {
            info = info.extend(Info::new().add_title("CONVERSATION").add_key("ID"));
        }

        if porcelain {
            self.writeln(Porcelain::from(&info).into_long().skip(1))?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }
}

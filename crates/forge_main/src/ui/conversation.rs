use std::collections::HashSet;

use anyhow::{Context, Result};
use colored::Colorize;
use forge_api::{API, Conversation, ConversationId, TextMessage};
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, ContextMessage, Role, TitleFormat};
use forge_select::ForgeWidget;
use futures::future;

use super::{ConversationDump, UI};
use crate::cli::ConversationCommand;
use crate::conversation_selector::ConversationSelector;
use crate::display_constants::markers;
use crate::info::Info;
use crate::porcelain::Porcelain;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    pub(super) async fn handle_conversation_command(
        &mut self,
        conversation_group: crate::cli::ConversationCommandGroup,
    ) -> anyhow::Result<()> {
        match conversation_group.command {
            ConversationCommand::List { porcelain } => {
                self.on_show_conversations(porcelain).await?;
            }
            ConversationCommand::New => {
                self.handle_generate_conversation_id().await?;
            }
            ConversationCommand::Dump { id, html } => {
                self.validate_conversation_exists(&id).await?;

                let original_id = self.state.conversation_id;
                self.state.conversation_id = Some(id);

                self.spinner.start(Some("Dumping"))?;
                self.on_dump(html).await?;

                self.state.conversation_id = original_id;
            }
            ConversationCommand::Compact { id } => {
                self.validate_conversation_exists(&id).await?;

                let original_id = self.state.conversation_id;
                self.state.conversation_id = Some(id);

                self.spinner.start(Some("Compacting"))?;
                self.on_compaction().await?;

                self.state.conversation_id = original_id;
            }
            ConversationCommand::Delete { id } => {
                let conversation_id =
                    ConversationId::parse(&id).context(format!("Invalid conversation ID: {id}"))?;

                self.validate_conversation_exists(&conversation_id).await?;

                self.on_conversation_delete(conversation_id).await?;
            }
            ConversationCommand::Retry { id } => {
                self.validate_conversation_exists(&id).await?;

                let original_id = self.state.conversation_id;
                self.state.conversation_id = Some(id);

                self.spinner.start(None)?;
                self.on_message(None).await?;

                self.state.conversation_id = original_id;
            }
            ConversationCommand::Resume { id } => {
                self.validate_conversation_exists(&id).await?;

                self.state.conversation_id = Some(id);
                self.writeln_title(TitleFormat::info(format!("Resumed conversation: {id}")))?;
                // Interactive mode will be handled by the main loop
            }
            ConversationCommand::Show { id, md } => {
                let conversation = self.validate_conversation_exists(&id).await?;

                self.on_show_last_message(conversation, md).await?;
            }
            ConversationCommand::Info { id } => {
                let conversation = self.validate_conversation_exists(&id).await?;

                self.on_show_conv_info(conversation).await?;
            }
            ConversationCommand::Stats { id, porcelain } => {
                let conversation = self.validate_conversation_exists(&id).await?;

                self.on_show_conv_stats(conversation, porcelain).await?;
            }
            ConversationCommand::Clone { id, porcelain } => {
                let conversation = self.validate_conversation_exists(&id).await?;

                self.spinner.start(Some("Cloning"))?;
                self.on_clone_conversation(conversation, porcelain).await?;
                self.spinner.stop(None)?;
            }
            ConversationCommand::Rename { id, name } => {
                self.validate_conversation_exists(&id).await?;

                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Please provide a name for the conversation."
                    ));
                }
                self.api.rename_conversation(&id, name.clone()).await?;
                self.writeln_title(TitleFormat::info(format!(
                    "Conversation renamed to '{}'",
                    name.bold()
                )))?;
            }
        }
        Ok(())
    }

    pub(super) async fn validate_conversation_exists(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Conversation> {
        let conversation = self.api.conversation(conversation_id).await?;

        conversation.ok_or_else(|| anyhow::anyhow!("Conversation '{conversation_id}' not found"))
    }

    pub(super) async fn on_conversation_delete(
        &mut self,
        conversation_id: ConversationId,
    ) -> anyhow::Result<()> {
        self.spinner.start(Some("Deleting conversation"))?;
        self.api.delete_conversation(&conversation_id).await?;
        self.spinner.stop(None)?;
        self.writeln_title(TitleFormat::debug(format!(
            "Successfully deleted conversation '{}'",
            conversation_id
        )))?;
        Ok(())
    }

    pub(super) async fn list_conversations(&mut self) -> anyhow::Result<()> {
        self.spinner.start(Some("Loading Conversations"))?;
        let max_conversations = self.config.max_conversations;
        let conversations = self.api.get_conversations(Some(max_conversations)).await?;
        let conversations = Self::user_initiated_conversations(conversations);
        self.spinner.stop(None)?;

        if conversations.is_empty() {
            self.writeln_title(TitleFormat::error(
                "No conversations found in this workspace.",
            ))?;
            return Ok(());
        }

        if let Some(conversation) = ConversationSelector::select_conversation(
            &conversations,
            self.state.conversation_id,
            None,
        )
        .await?
        {
            let conversation_id = conversation.id;
            self.state.conversation_id = Some(conversation_id);

            // Show conversation content
            self.on_show_last_message(conversation, false).await?;

            // Print log about conversation switching
            self.writeln_title(TitleFormat::info(format!(
                "Switched to conversation {}",
                conversation_id.into_string().bold()
            )))?;

            // Show conversation info
            self.on_info(false, Some(conversation_id)).await?;
        }
        Ok(())
    }

    pub(super) async fn on_show_conversations(&mut self, porcelain: bool) -> anyhow::Result<()> {
        let max_conversations = self.config.max_conversations;
        let conversations = self.api.get_conversations(Some(max_conversations)).await?;
        let conversations = Self::user_initiated_conversations(conversations);

        if conversations.is_empty() {
            return Ok(());
        }

        let mut info = Info::new();

        for conv in conversations.into_iter() {
            if conv.context.is_none() {
                continue;
            }

            let title = conv
                .title
                .as_deref()
                .map(|t| t.to_string())
                .unwrap_or_else(|| markers::EMPTY.to_string());

            // Format time using humantime library (same as conversation_selector.rs)
            let duration = chrono::Utc::now().signed_duration_since(
                conv.metadata.updated_at.unwrap_or(conv.metadata.created_at),
            );
            let duration =
                std::time::Duration::from_secs((duration.num_minutes() * 60).max(0) as u64);
            let time_ago = if duration.is_zero() {
                "now".to_string()
            } else {
                format!("{} ago", humantime::format_duration(duration))
            };

            // Add conversation: Title=<title>, Updated=<time_ago>, with ID as section title
            info = info
                .add_title(conv.id)
                .add_key_value("Title", title)
                .add_key_value("Updated", time_ago);
        }

        // In porcelain mode, skip the top-level "SESSIONS" title
        if porcelain {
            let porcelain = Porcelain::from(&info)
                .drop_col(3)
                .truncate(1, 60)
                .uppercase_headers();
            self.writeln(porcelain)?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    pub(super) fn user_initiated_conversations(
        conversations: Vec<Conversation>,
    ) -> Vec<Conversation> {
        let related_ids: HashSet<ConversationId> = conversations
            .iter()
            .flat_map(Conversation::related_conversation_ids)
            .collect();

        conversations
            .into_iter()
            .filter(|conversation| {
                conversation
                    .context
                    .as_ref()
                    .and_then(|context| context.initiator.as_deref())
                    .is_none_or(|initiator| initiator == "user")
                    && !related_ids.contains(&conversation.id)
            })
            .collect()
    }

    pub(super) async fn on_compaction(&mut self) -> Result<(), anyhow::Error> {
        let conversation_id = self.init_conversation().await?;
        let compaction_result = self.api.compact_conversation(&conversation_id).await?;
        let token_reduction = compaction_result.token_reduction_percentage();
        let message_reduction = compaction_result.message_reduction_percentage();
        let content = TitleFormat::action(format!(
            "Context size reduced by {token_reduction:.1}% (tokens), {message_reduction:.1}% (messages)"
        ));
        self.writeln_title(content)?;
        Ok(())
    }

    pub(super) async fn handle_delete_conversation(&mut self) -> anyhow::Result<()> {
        let conversation_id = self.init_conversation().await?;
        self.on_conversation_delete(conversation_id).await?;
        Ok(())
    }

    pub(super) async fn handle_rename_conversation(&mut self, name: String) -> anyhow::Result<()> {
        let conversation_id = self.init_conversation().await?;
        self.api
            .rename_conversation(&conversation_id, name.clone())
            .await?;
        self.writeln_title(TitleFormat::info(format!(
            "Conversation renamed to '{}'",
            name.bold()
        )))?;
        Ok(())
    }

    /// Clones a conversation (current or selected) and switches to the clone.
    ///
    /// # Arguments
    /// * `id` - Optional conversation ID to clone. If `None`, the current
    ///   conversation is used; if no active conversation, an interactive picker
    ///   is shown.
    pub(super) async fn on_slash_clone(&mut self, id: Option<String>) -> anyhow::Result<()> {
        let target_id = if let Some(id_str) = id {
            ConversationId::parse(&id_str)
                .map_err(|_| anyhow::anyhow!("Invalid conversation ID: {id_str}"))?
        } else {
            // Show conversation picker
            let conversations = self
                .api
                .get_conversations(Some(self.config.max_conversations))
                .await?;

            if conversations.is_empty() {
                self.writeln_title(TitleFormat::error(
                    "No conversations found. Start a conversation first.",
                ))?;
                return Ok(());
            }

            let selected = ConversationSelector::select_conversation(
                &conversations,
                self.state.conversation_id,
                None,
            )
            .await?;

            match selected {
                Some(conv) => conv.id,
                None => return Ok(()),
            }
        };

        // Fetch the conversation to clone
        let original = self
            .api
            .conversation(&target_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Conversation '{target_id}' not found"))?;

        let original_id = original.id;

        // Create the clone
        let new_id = ConversationId::generate();
        let mut cloned = original;
        cloned.id = new_id;
        self.api.upsert_conversation(cloned).await?;

        // Switch to the cloned conversation
        self.state.conversation_id = Some(new_id);

        self.writeln_title(
            TitleFormat::info("Cloned").sub_title(format!("[{original_id} → {new_id}]")),
        )?;

        Ok(())
    }

    /// Renames any conversation interactively or by explicit ID and name.
    ///
    /// # Arguments
    /// * `args` - Optional `"<id> <name>"` string. If `None`, shows a
    ///   conversation picker and prompts for a new name.
    pub(super) async fn on_slash_conversation_rename(
        &mut self,
        args: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(args) = args {
            // Parse as "<id> <name>"
            let mut parts = args.splitn(2, ' ');
            let id_str = parts.next().unwrap_or("").trim();
            let name = parts.next().unwrap_or("").trim();

            if id_str.is_empty() || name.is_empty() {
                return Err(anyhow::anyhow!("Usage: :conversation-rename <id> <name>"));
            }

            let conversation_id = ConversationId::parse(id_str)
                .map_err(|_| anyhow::anyhow!("Invalid conversation ID: {id_str}"))?;

            self.api
                .rename_conversation(&conversation_id, name.to_string())
                .await?;
            self.writeln_title(TitleFormat::info(format!(
                "Conversation '{}' renamed to '{}'",
                conversation_id.into_string().bold(),
                name.bold()
            )))?;
        } else {
            // Interactive: show picker then prompt for new name
            let conversations = self
                .api
                .get_conversations(Some(self.config.max_conversations))
                .await?;

            if conversations.is_empty() {
                self.writeln_title(TitleFormat::error("No conversations found."))?;
                return Ok(());
            }

            let selected = ConversationSelector::select_conversation(
                &conversations,
                self.state.conversation_id,
                None,
            )
            .await?;

            if let Some(conv) = selected {
                let name_result = ForgeWidget::input("New name").allow_empty(false).prompt()?;

                if let Some(name) = name_result
                    && !name.is_empty()
                {
                    self.api.rename_conversation(&conv.id, name.clone()).await?;
                    self.writeln_title(TitleFormat::info(format!(
                        "Conversation renamed to '{}'",
                        name.bold()
                    )))?;
                }
            }
        }

        Ok(())
    }

    /// Fetches related conversations for a given conversation in parallel.
    ///
    /// Returns a vector of related conversations that could be successfully
    /// fetched.
    pub(super) async fn fetch_related_conversations(
        &self,
        conversation: &Conversation,
    ) -> Vec<Conversation> {
        let related_ids = conversation.related_conversation_ids();

        // Fetch all related conversations in parallel
        let related_futures: Vec<_> = related_ids
            .iter()
            .map(|id| {
                let api = self.api.clone();
                let id = *id;
                async move { api.conversation(&id).await }
            })
            .collect();

        future::join_all(related_futures)
            .await
            .into_iter()
            .filter_map(|result| result.ok().flatten())
            .collect()
    }

    /// Modified version of handle_dump that supports HTML format
    pub(super) async fn on_dump(&mut self, html: bool) -> Result<()> {
        if let Some(conversation_id) = self.state.conversation_id {
            let conversation = self.api.conversation(&conversation_id).await?;
            if let Some(conversation) = conversation {
                let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");

                // Collect related conversations from agent tool calls
                let related_conversations = self.fetch_related_conversations(&conversation).await;

                if html {
                    // Create a single HTML with all conversations
                    let html_content = if related_conversations.is_empty() {
                        // No related conversations, just render the main one
                        conversation.to_html()
                    } else {
                        // Render main conversation with related conversations in the same HTML
                        conversation.to_html_with_related(&related_conversations)
                    };

                    let path = format!("{timestamp}-dump.html");
                    tokio::fs::write(path.as_str(), &html_content).await?;

                    let subtitle = if related_conversations.is_empty() {
                        path.to_string()
                    } else {
                        format!("{} (+ {} related)", path, related_conversations.len())
                    };

                    self.writeln_title(
                        TitleFormat::action("Conversation HTML dump created".to_string())
                            .sub_title(subtitle),
                    )?;

                    if self.config.auto_open_dump {
                        open::that(path.as_str()).ok();
                    }
                } else {
                    let dump_data = ConversationDump {
                        conversation: conversation.clone(),
                        related_conversations: related_conversations.clone(),
                    };

                    let path = format!("{timestamp}-dump.json");
                    let content = serde_json::to_string_pretty(&dump_data)?;
                    tokio::fs::write(path.as_str(), content).await?;

                    let subtitle = if related_conversations.is_empty() {
                        path.to_string()
                    } else {
                        format!("{} (+ {} related)", path, related_conversations.len())
                    };

                    self.writeln_title(
                        TitleFormat::action("Conversation JSON dump created".to_string())
                            .sub_title(subtitle),
                    )?;

                    if self.config.auto_open_dump {
                        open::that(path.as_str()).ok();
                    }
                };
            } else {
                return Err(anyhow::anyhow!("Could not create dump"))
                    .context(format!("Conversation: {conversation_id} was not found"));
            }
        } else {
            return Err(anyhow::anyhow!("No conversation initiated yet"))
                .context("Could not create dump");
        }
        Ok(())
    }

    pub(super) async fn on_show_conv_info(
        &mut self,
        conversation: Conversation,
    ) -> anyhow::Result<()> {
        self.spinner.start(Some("Loading Summary"))?;

        let info = Info::default().extend(&conversation);
        self.writeln(info)?;
        self.spinner.stop(None)?;

        Ok(())
    }

    pub(super) async fn on_show_conv_stats(
        &mut self,
        conversation: Conversation,
        porcelain: bool,
    ) -> anyhow::Result<()> {
        let mut info = Info::new().add_title("CONVERSATION");

        // Add conversation ID
        info = info.add_key_value("ID", conversation.id.to_string());

        // Calculate duration
        let created_at = conversation.metadata.created_at;
        let updated_at = conversation.metadata.updated_at.unwrap_or(created_at);
        let duration = updated_at.signed_duration_since(created_at);

        // Format duration
        let duration_str = if duration.num_hours() > 0 {
            format!("{}h {}m", duration.num_hours(), duration.num_minutes() % 60)
        } else if duration.num_minutes() > 0 {
            format!(
                "{}m {}s",
                duration.num_minutes(),
                duration.num_seconds() % 60
            )
        } else {
            format!("{}s", duration.num_seconds())
        };

        info = info.add_key_value("Total Duration", duration_str);

        // Add message statistics if context exists
        if let Some(context) = &conversation.context {
            info = info
                .add_key_value("Total Messages", context.total_messages().to_string())
                .add_key_value("User Messages", context.user_message_count().to_string())
                .add_key_value(
                    "Assistant Messages",
                    context.assistant_message_count().to_string(),
                )
                .add_key_value("Tool Calls", context.tool_call_count().to_string());
        }

        // Add token usage if available
        if let Some(usage) = conversation.usage().as_ref() {
            info = info
                .add_title("TOKEN")
                .add_key_value("Prompt Tokens", usage.prompt_tokens.to_string())
                .add_key_value("Completion Tokens", usage.completion_tokens.to_string())
                .add_key_value("Total Tokens", usage.total_tokens.to_string());
        }

        if let Some(cost) = conversation.accumulated_cost() {
            info = info.add_key_value("Cost", format!("${cost:.4}"));
        }

        if porcelain {
            use convert_case::Case;
            self.writeln(
                Porcelain::from(&info)
                    .into_long()
                    .skip(1)
                    .to_case(&[0, 1], Case::Snake)
                    .sort_by(&[0, 1]),
            )?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Clones a conversation with a new ID
    ///
    /// # Arguments
    /// * `original` - The conversation to clone
    /// * `porcelain` - If true, output only the new conversation ID
    pub(super) async fn on_clone_conversation(
        &mut self,
        original: Conversation,
        porcelain: bool,
    ) -> anyhow::Result<()> {
        // Create a new conversation with a new ID but same content
        let new_id = ConversationId::generate();
        let mut cloned = original.clone();
        cloned.id = new_id;

        // Upsert the cloned conversation
        self.api.upsert_conversation(cloned.clone()).await?;

        // Output based on format
        if porcelain {
            println!("{new_id}");
        } else {
            self.writeln_title(
                TitleFormat::info("Cloned").sub_title(format!("[{} → {}]", original.id, cloned.id)),
            )?;
        }

        Ok(())
    }

    /// Shows the last message from a conversation
    ///
    /// When `md` is true, the raw markdown content is printed without
    /// rendering. When `md` is false, the content is rendered through the
    /// markdown renderer.
    ///
    /// # Errors
    /// - If the conversation doesn't exist
    /// - If the conversation has no messages
    pub(super) async fn on_show_last_message(
        &mut self,
        conversation: Conversation,
        md: bool,
    ) -> Result<()> {
        let context = conversation
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Conversation has no context"))?;

        // Find the last assistant message
        let message = context.messages.iter().rev().find_map(|msg| match &**msg {
            ContextMessage::Text(TextMessage { content, role: Role::Assistant, .. }) => {
                Some(content)
            }
            _ => None,
        });

        // Format and display the message using the message_display module
        if let Some(message) = message {
            if md {
                self.writeln(message)?;
            } else {
                self.writeln(self.markdown.render(message))?;
            }
        }

        Ok(())
    }
}

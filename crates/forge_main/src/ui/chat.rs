use anyhow::Result;
use forge_api::{API, ChatRequest, ChatResponse, Event, InterruptionReason};
use forge_config::ForgeConfig;
use forge_domain::{ChatResponseContent, ConsoleWriter, TitleFormat, UserCommand};
use forge_select::ForgeWidget;
use forge_tracker::ToolCallPayload;
use tokio_stream::StreamExt;

use super::UI;
use crate::info::Info;
use crate::stream_renderer::StreamingWriter;
use crate::title_display::TitleDisplayExt;
use crate::tracker;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    // Handle dispatching events from the CLI
    pub(super) async fn handle_dispatch(&mut self, json: String) -> Result<()> {
        // Initialize the conversation
        let conversation_id = self.init_conversation().await?;

        // Parse the JSON to determine the event name and value
        let event: UserCommand = serde_json::from_str(&json)?;

        // Create the chat request with the event
        let chat = ChatRequest::new(event.into(), conversation_id);

        self.on_chat(chat).await
    }

    pub(super) async fn on_message(&mut self, content: Option<String>) -> Result<()> {
        let conversation_id = self.init_conversation().await?;

        if self.config.auto_install_vscode_extension {
            self.install_vscode_extension();
        }

        // Track if content was provided to decide whether to use piped input as
        // additional context
        let has_content = content.is_some();

        // Create a ChatRequest with the appropriate event type
        let mut event = match content {
            Some(text) => Event::new(text),
            None => Event::empty(),
        };

        // Only use CLI piped_input as additional context when BOTH --prompt and piped
        // input are provided. This handles the case: `echo "context" | forge -p
        // "question"` where piped input provides context and --prompt provides
        // the actual question.
        //
        // When only piped input is provided (no --prompt), it's already used as the
        // main content (passed via the `content` parameter). We must NOT add it again
        // as additional_context, otherwise the input appears twice in the
        // conversation. We detect this by checking if cli.prompt exists - if it
        // does, the content came from --prompt and piped input should be
        // additional context.
        let piped_input = self.cli.piped_input.clone();
        let has_explicit_prompt = self.cli.prompt.is_some();
        if let Some(piped) = piped_input
            && has_content
            && has_explicit_prompt
        {
            event = event.additional_context(piped);
        }

        // Create the chat request with the event
        let chat = ChatRequest::new(event, conversation_id);

        self.on_chat(chat).await
    }

    pub(super) async fn on_chat(&mut self, chat: ChatRequest) -> Result<()> {
        let mut stream = self.api.chat(chat).await?;

        // Always use streaming content writer
        let mut writer = StreamingWriter::new(self.spinner.clone(), self.api.clone());

        while let Some(message) = stream.next().await {
            match message {
                Ok(message) => self.handle_chat_response(message, &mut writer).await?,
                Err(err) => {
                    writer.finish()?;
                    self.spinner.stop(None)?;
                    self.spinner.reset();
                    return Err(err);
                }
            }
        }

        writer.finish()?;
        self.spinner.stop(None)?;
        self.spinner.reset();

        Ok(())
    }

    pub(super) async fn handle_chat_response(
        &mut self,
        message: ChatResponse,
        writer: &mut StreamingWriter<A>,
    ) -> Result<()> {
        if message.is_empty() {
            return Ok(());
        }
        match message {
            ChatResponse::TaskMessage { content } => match content {
                ChatResponseContent::ToolInput(title) => {
                    writer.finish()?;
                    self.writeln(title.display())?;
                }
                ChatResponseContent::ToolOutput(text) => {
                    writer.finish()?;
                    self.writeln(text)?;
                }
                ChatResponseContent::Markdown { text, partial: _ } => {
                    writer.write(&text)?;
                }
            },
            ChatResponse::ToolCallStart { tool_call, notifier } => {
                // Scope guard to ensure notification happens even on error.
                // If writer.finish() or spinner.stop() fails, the guard's drop
                // will still notify orch, preventing the deadlock.
                struct NotifyGuard<'a>(&'a tokio::sync::Notify);
                impl<'a> Drop for NotifyGuard<'a> {
                    fn drop(&mut self) {
                        self.0.notify_one();
                    }
                }
                let _guard = NotifyGuard(&notifier);

                writer.finish()?;

                // Stop spinner only for tools that require stdout/stderr access
                if tool_call.requires_stdout() {
                    self.spinner.stop(None)?;
                }

                // Notify orch that the UI has rendered the tool header.
                // Orch awaits this before executing the tool, preventing tool
                // stdout from appearing before the tool name is printed.
                drop(_guard);
            }
            ChatResponse::ToolCallEnd(toolcall_result) => {
                // Only track toolcall name in case of success else track the error.
                let payload = if toolcall_result.is_error() {
                    let mut r = ToolCallPayload::new(toolcall_result.name.to_string());
                    if let Some(cause) = toolcall_result.output.as_str() {
                        r = r.with_cause(cause.to_string());
                    }
                    r
                } else {
                    ToolCallPayload::new(toolcall_result.name.to_string())
                };
                tracker::tool_call(payload);

                self.spinner.start(None)?;
                if !self.cli.verbose {
                    return Ok(());
                }
            }
            ChatResponse::RetryAttempt { cause, duration: _ } => {
                if !self
                    .config
                    .retry
                    .as_ref()
                    .is_some_and(|r| r.suppress_errors)
                {
                    writer.finish()?;
                    self.spinner.start(Some("Retrying"))?;
                    self.writeln_title(TitleFormat::error(cause.as_str()))?;
                }
            }
            ChatResponse::Interrupt { reason } => {
                writer.finish()?;
                self.spinner.stop(None)?;

                let title = match reason {
                    InterruptionReason::MaxRequestPerTurnLimitReached { limit } => {
                        format!("Maximum request ({limit}) per turn achieved")
                    }
                    InterruptionReason::MaxToolFailurePerTurnLimitReached { limit, .. } => {
                        format!("Maximum tool failure limit ({limit}) reached for this turn")
                    }
                };

                self.writeln_title(TitleFormat::action(title))?;
                let continued = self.should_continue().await?;
                if !continued && let Some(conversation_id) = self.state.conversation_id {
                    self.writeln_title(
                        TitleFormat::debug("Finished").sub_title(conversation_id.into_string()),
                    )?;
                }
            }
            ChatResponse::TaskReasoning { content } => {
                writer.write_dimmed(&content)?;
            }
            ChatResponse::TaskComplete => {
                writer.finish()?;
                if let Some(conversation_id) = self.state.conversation_id {
                    self.writeln_title(
                        TitleFormat::debug("Finished").sub_title(conversation_id.into_string()),
                    )?;
                }
                if let Some(format) = self.config.auto_dump.clone() {
                    let html = matches!(format, forge_config::AutoDumpFormat::Html);
                    self.on_dump(html).await?;
                }
            }
        }
        Ok(())
    }

    pub(super) async fn should_continue(&mut self) -> anyhow::Result<bool> {
        let should_continue = ForgeWidget::confirm("Do you want to continue anyway?")
            .with_default(true)
            .prompt()?;

        if should_continue.unwrap_or(false) {
            self.spinner.start(None)?;
            Box::pin(self.on_message(None)).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) async fn on_custom_event(&mut self, event: Event) -> Result<()> {
        let conversation_id = self.init_conversation().await?;
        let chat = ChatRequest::new(event, conversation_id);
        self.on_chat(chat).await
    }

    pub(super) async fn on_usage(&mut self) -> anyhow::Result<()> {
        self.spinner.start(Some("Loading Usage"))?;

        // Get usage from current conversation if available
        let conversation_usage = if let Some(conversation_id) = &self.state.conversation_id {
            self.api
                .conversation(conversation_id)
                .await
                .ok()
                .flatten()
                .and_then(|conv| conv.accumulated_usage())
        } else {
            None
        };

        let info = if let Some(usage) = conversation_usage {
            Info::from(&usage)
        } else {
            Info::new()
        };


        self.writeln(info)?;
        self.spinner.stop(None)?;
        Ok(())
    }
}

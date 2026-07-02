use std::path::PathBuf;

use anyhow::Context;
use forge_api::API;
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, TitleFormat};
use forge_select::ForgeWidget;
use tokio_stream::StreamExt;

use super::UI;
use crate::info::Info;
use crate::porcelain::Porcelain;
use crate::sync_display::SyncProgressDisplay;
use crate::title_display::TitleDisplayExt;
use crate::utils::humanize_time;

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    /// Syncs (indexes) the workspace at the given path, optionally
    /// initializing it first when `init` is set.
    pub(super) async fn on_index(&mut self, path: PathBuf, init: bool) -> anyhow::Result<()> {
        use forge_domain::SyncProgress;
        use forge_spinner::ProgressBarManager;

        // Check if auth already exists and create if needed
        if !self.api.is_authenticated().await? {
            self.init_forge_services().await?;
        }

        // When init is set, check if the workspace is already initialized
        // via get_workspace_info before calling init, so we only initialize
        // when a workspace does not yet exist for the given path.
        if init {
            let workspace_info = self.api.get_workspace_info(path.clone()).await?;
            if workspace_info.is_none() {
                self.on_workspace_init(path.clone(), false).await?;
                // If the workspace still does not exist after init (e.g. user
                // declined the consent prompt), abort the sync.
                let workspace_info = self.api.get_workspace_info(path.clone()).await?;
                if workspace_info.is_none() {
                    return Ok(());
                }
            }
        }

        let mut stream = self.api.sync_workspace(path.clone()).await?;
        let mut progress_bar = ProgressBarManager::default();

        while let Some(event) = stream.next().await {
            match event {
                Ok(ref progress @ SyncProgress::Completed { .. }) => {
                    progress_bar.set_position(100)?;
                    progress_bar.stop(None).await?;
                    if let Some(msg) = progress.message() {
                        self.writeln_title(TitleFormat::debug(msg))?;
                    }
                }
                Ok(ref progress @ SyncProgress::Syncing { .. }) => {
                    if !progress_bar.is_active() {
                        progress_bar.start(100, "Indexing workspace")?;
                    }
                    if let Some(msg) = progress.message() {
                        progress_bar.set_message(&msg)?;
                    }
                    if let Some(weight) = progress.weight() {
                        progress_bar.set_position(weight)?;
                    }
                }
                Ok(ref progress) => {
                    if let Some(msg) = progress.message() {
                        self.writeln_title(TitleFormat::debug(msg))?;
                    }
                }
                Err(e) => {
                    progress_bar.stop(None).await?;
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// Queries the indexed workspace and displays matching results.
    pub(super) async fn on_query(
        &mut self,
        path: PathBuf,
        params: forge_domain::SearchParams<'_>,
    ) -> anyhow::Result<()> {
        self.spinner.start(Some("Searching workspace..."))?;

        let results = match self.api.query_workspace(path.clone(), params).await {
            Ok(results) => results,
            Err(e) => {
                self.spinner.stop(None)?;
                return Err(e);
            }
        };

        self.spinner.stop(None)?;

        let mut info = Info::new().add_title(format!("FILES [{} RESULTS]", results.len()));

        for result in results.iter() {
            match &result.node {
                forge_domain::NodeData::FileChunk(chunk) => {
                    info = info.add_key_value(
                        "File",
                        format!(
                            "{}:{}-{}",
                            chunk.file_path, chunk.start_line, chunk.end_line
                        ),
                    );
                }
                forge_domain::NodeData::File(file) => {
                    info = info.add_key_value("File", format!("{} (full file)", file.file_path));
                }
                forge_domain::NodeData::FileRef(file_ref) => {
                    info =
                        info.add_key_value("File", format!("{} (reference)", file_ref.file_path));
                }
                forge_domain::NodeData::Note(note) => {
                    info = info.add_key_value("Note", &note.content);
                }
                forge_domain::NodeData::Task(task) => {
                    info = info.add_key_value("Task", &task.task);
                }
            }
        }

        self.writeln(info)?;

        Ok(())
    }

    /// Helper function to format workspace information consistently
    fn format_workspace_info(workspace: &forge_domain::WorkspaceInfo, is_active: bool) -> Info {
        let updated_time = workspace
            .last_updated
            .map_or("NEVER".to_string(), humanize_time);

        let mut info = Info::new();

        let title = if is_active {
            "Workspace [Current]".to_string()
        } else {
            "Workspace".to_string()
        };
        info = info.add_title(title);

        info.add_key_value("ID", workspace.workspace_id.to_string())
            .add_key_value("Path", workspace.working_dir.to_string())
            .add_key_value("Created At", humanize_time(workspace.created_at))
            .add_key_value("Updated At", updated_time)
    }

    /// Lists all known workspaces, marking the active one.
    pub(super) async fn on_list_workspaces(&mut self, porcelain: bool) -> anyhow::Result<()> {
        if !porcelain {
            self.spinner.start(Some("Fetching workspaces..."))?;
        }

        // Fetch workspaces and current workspace info in parallel
        let env = self.api.environment();
        let (workspaces_result, current_workspace_result) = tokio::join!(
            self.api.list_workspaces(),
            self.api.get_workspace_info(env.cwd)
        );

        match workspaces_result {
            Ok(workspaces) => {
                if !porcelain {
                    self.spinner.stop(None)?;
                }

                // Get active workspace ID if current workspace info is available
                let current_workspace = current_workspace_result.ok().flatten();
                let active_workspace_id = current_workspace.as_ref().map(|ws| &ws.workspace_id);

                // Build Info object once
                let mut info = Info::new();

                for workspace in &workspaces {
                    let is_active = active_workspace_id == Some(&workspace.workspace_id);
                    info = info.extend(Self::format_workspace_info(workspace, is_active));
                }

                // Output based on mode
                if porcelain {
                    // Skip header row in porcelain mode (consistent with conversation list)
                    self.writeln(Porcelain::from(info).skip(1).drop_cols(&[0, 4, 5]))?;
                } else {
                    self.writeln(info)?;
                }

                Ok(())
            }
            Err(e) => {
                self.spinner.stop(None)?;
                Err(e)
            }
        }
    }

    /// Displays workspace information for a given path.
    pub(super) async fn on_workspace_info(&mut self, path: PathBuf) -> anyhow::Result<()> {
        self.spinner.start(Some("Fetching workspace info..."))?;

        // Fetch workspace info and status in parallel
        let (workspace, statuses) = tokio::try_join!(
            self.api.get_workspace_info(path.clone()),
            self.api.get_workspace_status(path)
        )?;

        self.spinner.stop(None)?;

        match workspace {
            Some(workspace) => {
                // When viewing a specific workspace's info, it's implicitly the active one
                let mut info = Self::format_workspace_info(&workspace, true);

                // Add sync status summary if available

                use forge_domain::SyncStatus;

                let in_sync = statuses
                    .iter()
                    .filter(|s| s.status == SyncStatus::InSync)
                    .count();
                let modified = statuses
                    .iter()
                    .filter(|s| s.status == SyncStatus::Modified)
                    .count();
                let added = statuses
                    .iter()
                    .filter(|s| s.status == SyncStatus::New)
                    .count();
                let deleted = statuses
                    .iter()
                    .filter(|s| s.status == SyncStatus::Deleted)
                    .count();
                let failed = statuses
                    .iter()
                    .filter(|s| s.status == SyncStatus::Failed)
                    .count();

                // Add sync status section
                info = info.add_title("Sync Status");
                info = info.add_key_value("Total Files", statuses.len().to_string());
                if in_sync > 0 {
                    info = info.add_key_value("In Sync", in_sync.to_string());
                }
                if modified > 0 {
                    info = info.add_key_value("Modified", modified.to_string());
                }
                if added > 0 {
                    info = info.add_key_value("Added", added.to_string());
                }
                if deleted > 0 {
                    info = info.add_key_value("Deleted", deleted.to_string());
                }
                if failed > 0 {
                    info = info.add_key_value("Failed", failed.to_string());
                }

                self.writeln(info)
            }
            None => self.writeln_to_stderr(
                TitleFormat::error("No workspace found")
                    .display()
                    .to_string(),
            ),
        }
    }

    /// Deletes the given workspaces by ID.
    pub(super) async fn on_delete_workspaces(
        &mut self,
        workspace_ids: Vec<String>,
    ) -> anyhow::Result<()> {
        if workspace_ids.is_empty() {
            anyhow::bail!("At least one workspace ID is required");
        }

        // Parse all workspace IDs
        let parsed_ids: Vec<forge_domain::WorkspaceId> = workspace_ids
            .iter()
            .map(|id| {
                forge_domain::WorkspaceId::from_string(id)
                    .with_context(|| format!("Invalid workspace ID format: {}", id))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let total = parsed_ids.len();
        self.spinner.start(Some(&format!(
            "Deleting {} workspace{}...",
            total,
            if total > 1 { "s" } else { "" }
        )))?;

        match self.api.delete_workspaces(parsed_ids.clone()).await {
            Ok(()) => {
                self.spinner.stop(None)?;
                for id in &parsed_ids {
                    self.writeln_title(TitleFormat::debug(format!(
                        "Successfully deleted workspace {}",
                        id
                    )))?;
                }
                Ok(())
            }
            Err(e) => {
                self.spinner.stop(None)?;
                Err(e)
            }
        }
    }

    /// Displays sync status for all files in the workspace.
    pub(super) async fn on_workspace_status(
        &mut self,
        path: PathBuf,
        porcelain: bool,
    ) -> anyhow::Result<()> {
        use forge_domain::SyncStatus;

        if !porcelain {
            self.spinner.start(Some("Checking file status..."))?;
        }

        let mut statuses = self.api.get_workspace_status(path.clone()).await?;
        statuses.sort_by(|a, b| a.status.cmp(&b.status));

        if !porcelain {
            self.spinner.stop(None)?;
        }

        // Calculate out of sync count
        let out_of_sync = statuses
            .iter()
            .filter(|s| {
                s.status == SyncStatus::Modified
                    || s.status == SyncStatus::New
                    || s.status == SyncStatus::Deleted
                    || s.status == SyncStatus::Failed
            })
            .count();

        // When all files are in sync, show a simple log message
        if out_of_sync == 0 {
            if porcelain {
                // In porcelain mode, output empty result
                self.writeln(
                    Porcelain::from(Info::new())
                        .into_long()
                        .set_headers(["STATUS", "FILE"])
                        .uppercase_headers(),
                )?;
            } else {
                // Show log info message when all files are in sync
                self.writeln_title(TitleFormat::info(format!(
                    "All {} files are in sync",
                    statuses.len()
                )))?;
            }
            return Ok(());
        }

        // Build file list info only when there are files out of sync
        let mut info = Info::new().add_title(format!("File Status [{} out of sync]", out_of_sync));

        // Add file list (skip in-sync files)
        for (status, label) in statuses.iter().filter_map(|status| match status.status {
            SyncStatus::InSync => None,
            SyncStatus::Modified => Some((status, "modified")),
            SyncStatus::New => Some((status, "added")),
            SyncStatus::Deleted => Some((status, "deleted")),
            SyncStatus::Failed => Some((status, "failed")),
        }) {
            info = info.add_key_value(&status.path, label);
        }

        // Output based on mode
        if porcelain {
            self.writeln(
                Porcelain::from(info)
                    .into_long()
                    .drop_col(0)
                    .swap_cols(0, 1)
                    .set_headers(["STATUS", "FILE"])
                    .sort_by(&[0])
                    .uppercase_headers(),
            )?;
        } else {
            self.writeln(info)?;
        }

        Ok(())
    }

    /// Initialize workspace for a directory without syncing files
    pub(super) async fn on_workspace_init(&mut self, path: PathBuf, yes: bool) -> anyhow::Result<()> {
        // Ask for user consent before syncing and sharing directory contents
        // with the ForgeCode Service.
        let display_path = path.display().to_string();

        let confirmed = if yes {
            Some(true)
        } else {
            ForgeWidget::confirm(format!(
                "This will sync and share the contents of '{}' with ForgeCode Services. Do you wish to continue?",
                display_path
            ))
            .with_default(true)
            .prompt()?
        };

        if !confirmed.unwrap_or(false) {
            self.writeln_title(TitleFormat::info("Workspace initialization cancelled"))?;
            return Ok(());
        }

        // Check if auth already exists and create if needed
        if !self.api.is_authenticated().await? {
            self.init_forge_services().await?;
        }

        self.spinner.start(Some("Initializing workspace"))?;

        let workspace_id = self.api.init_workspace(path.clone()).await?;

        self.spinner.stop(None)?;

        self.writeln_title(
            TitleFormat::info("Workspace initialized successfully")
                .sub_title(format!("{}", workspace_id)),
        )?;

        Ok(())
    }
}

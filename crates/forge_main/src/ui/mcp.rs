use colored::Colorize;
use console::style;
use forge_api::API;
use forge_config::ForgeConfig;
use forge_domain::{ConsoleWriter, TitleFormat};

use super::UI;
use crate::display_constants::status;
use crate::info::Info;
use crate::porcelain::Porcelain;

/// Formats an MCP server config for display, redacting sensitive information.
/// Returns the command/URL string only.
pub(super) fn format_mcp_server(server: &forge_domain::McpServerConfig) -> String {
    match server {
        forge_domain::McpServerConfig::Stdio(stdio) => {
            let mut output = format!("{} ", stdio.command);
            for arg in &stdio.args {
                output.push_str(&format!("{arg} "));
            }
            for key in stdio.env.keys() {
                output.push_str(&format!("{key}=*** "));
            }
            output.trim().to_string()
        }
        forge_domain::McpServerConfig::Http(http) => http.url.clone(),
    }
}

/// Formats HTTP headers for display, redacting values.
/// Returns None if there are no headers.
pub(super) fn format_mcp_headers(server: &forge_domain::McpServerConfig) -> Option<String> {
    match server {
        forge_domain::McpServerConfig::Stdio(_) => None,
        forge_domain::McpServerConfig::Http(http) => {
            if http.headers.is_empty() {
                None
            } else {
                Some(
                    http.headers
                        .keys()
                        .map(|k| format!("{k}=***"))
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            }
        }
    }
}

impl<A: API + ConsoleWriter + 'static, F: Fn(ForgeConfig) -> A + Send + Sync> UI<A, F> {
    /// Handle `mcp login <name>` command.
    ///
    /// Triggers the OAuth authentication flow for the specified MCP server.
    /// Uses the API layer which delegates to rmcp's OAuth state machine
    /// for metadata discovery, dynamic registration, PKCE, and token exchange.
    pub(super) async fn handle_mcp_login(&mut self, name: &str) -> anyhow::Result<()> {
        let server_name = forge_api::ServerName::from(name.to_string());
        let config = self.api.read_mcp_config(None).await?;
        let server = config.mcp_servers.get(&server_name);

        match server {
            Some(forge_domain::McpServerConfig::Http(http)) => {
                // Check auth status first
                let status = self.api.mcp_auth_status(&http.url).await?;
                if status == "authenticated" {
                    self.writeln_title(TitleFormat::info(
                        format!("MCP server '{}' is already authenticated. Use 'mcp logout {}' first to re-authenticate.", name, name)
                    ))?;
                    return Ok(());
                }

                // Force re-auth by removing any stale credentials
                let _ = self.api.mcp_logout(Some(&http.url)).await;

                // Run the OAuth flow (opens browser, waits for callback)
                match self.api.mcp_auth(&http.url).await {
                    Ok(()) => {
                        self.writeln_title(TitleFormat::info(format!(
                            "Successfully authenticated with MCP server '{}'",
                            name
                        )))?;
                        // Reload MCP to reconnect with new credentials
                        self.spinner.start(Some("Reloading MCPs"))?;
                        match self.api.reload_mcp().await {
                            Ok(()) => {
                                self.writeln_title(TitleFormat::info("MCP reloaded"))?;
                            }
                            Err(e) => {
                                self.writeln_title(TitleFormat::error(format!(
                                    "MCP reload failed: {}",
                                    e
                                )))?;
                            }
                        }
                    }
                    Err(e) => {
                        self.writeln_title(TitleFormat::error(format!(
                            "Authentication with MCP server '{}' failed: {}",
                            name, e
                        )))?;
                    }
                }
            }
            Some(_) => {
                self.writeln_title(TitleFormat::error(format!(
                    "MCP server '{}' is not an HTTP server (OAuth only applies to HTTP servers)",
                    name
                )))?;
            }
            None => {
                self.writeln_title(TitleFormat::error(format!(
                    "MCP server '{}' not found. Use 'mcp list' to see available servers.",
                    name
                )))?;
            }
        }
        Ok(())
    }

    /// Handle `mcp logout <name>` command.
    ///
    /// Removes stored OAuth credentials for the specified MCP server
    /// or all servers if "all" is specified.
    /// Automatically reloads MCPs after logout to reflect auth state change.
    pub(super) async fn handle_mcp_logout(&mut self, name: &str) -> anyhow::Result<()> {
        if name == "all" {
            self.api.mcp_logout(None).await?;
            self.writeln_title(TitleFormat::info("Removed all MCP OAuth credentials"))?;
        } else {
            let server_name = forge_api::ServerName::from(name.to_string());
            let config = self.api.read_mcp_config(None).await?;
            let server = config.mcp_servers.get(&server_name);

            match server {
                Some(forge_domain::McpServerConfig::Http(http)) => {
                    self.api.mcp_logout(Some(&http.url)).await?;
                    self.writeln_title(TitleFormat::info(format!(
                        "Removed OAuth credentials for MCP server '{}'",
                        name
                    )))?;
                }
                Some(_) => {
                    self.writeln_title(TitleFormat::error(format!(
                        "MCP server '{}' is not an HTTP server",
                        name
                    )))?;
                    return Ok(());
                }
                None => {
                    self.writeln_title(TitleFormat::error(format!(
                        "MCP server '{}' not found. Use 'mcp list' to see available servers.",
                        name
                    )))?;
                    return Ok(());
                }
            }
        }

        // Reload MCPs to reflect auth state change
        self.spinner.start(Some("Reloading MCPs"))?;
        self.api.reload_mcp().await?;
        self.writeln_title(TitleFormat::info("MCP reloaded"))?;

        Ok(())
    }

    /// Displays all MCP servers with their available tools
    pub(super) async fn on_show_mcp_servers(&mut self, porcelain: bool) -> anyhow::Result<()> {
        self.spinner.start(Some("Loading MCP servers"))?;
        let mcp_servers = self.api.read_mcp_config(None).await?;
        let all_tools = self.api.get_tools().await?;

        let mut info = Info::new();

        for (name, server) in mcp_servers.mcp_servers {
            let label = match server {
                forge_domain::McpServerConfig::Stdio(_) => "Command",
                forge_domain::McpServerConfig::Http(_) => "URL",
            };

            info = info
                .add_title(name.to_uppercase())
                .add_key_value("Type", server.server_type())
                .add_key_value(label, format_mcp_server(&server));

            // Add headers for HTTP servers if present
            if let Some(headers) = format_mcp_headers(&server) {
                info = info.add_key_value("Headers", headers);
            }

            if server.is_disabled() {
                info = info.add_key_value("Status", status::NO);
            }

            // Add tools for this MCP server
            if let Some(tools) = all_tools.mcp.get_servers().get(&name)
                && !tools.is_empty()
            {
                info = info.add_key_value("Tools", tools.len().to_string());
                for tool in tools {
                    info = info.add_value(tool.name.to_string());
                }
            }
        }

        if porcelain {
            self.writeln(Porcelain::from(&info).uppercase_headers().truncate(3, 60))?;
        } else {
            self.writeln(info)?;
        }

        // Show failed MCP servers
        if !porcelain && !all_tools.mcp.get_failures().is_empty() {
            self.writeln("MCP FAILURES\n".dimmed().bold())?;
            for error in all_tools.mcp.get_failures().values() {
                let error = style(error).red();
                self.writeln(error)?;
            }
        }

        Ok(())
    }
}

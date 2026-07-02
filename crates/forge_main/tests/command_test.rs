//! Unit tests for `crate::model`, included via `#[path]` from
//! `src/model.rs`. Not a standalone integration test (autotests is
//! disabled in Cargo.toml) because these tests exercise private items.

use std::fmt::Display;
use std::sync::{Arc, Mutex};

use colored::Colorize;
use console::strip_ansi_codes;
use forge_api::{
    AnyProvider, InputModality, Model, ModelId, ModelSource, ProviderId, ProviderResponse, Template,
};
use forge_domain::Provider;
use pretty_assertions::assert_eq;
use url::Url;

use super::*;
use crate::display_constants::markers;

/// Test-only wrapper for displaying models in selection menus
#[derive(Clone)]
struct CliModel(Model);

impl Display for CliModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.id)?;

        let mut info_parts = Vec::new();

        if let Some(limit) = self.0.context_length {
            if limit >= 1_000_000 {
                info_parts.push(format!("{}M", limit / 1_000_000));
            } else if limit >= 1000 {
                info_parts.push(format!("{}k", limit / 1000));
            } else {
                info_parts.push(format!("{limit}"));
            }
        }

        if self.0.tools_supported == Some(true) {
            info_parts.push("🛠️".to_string());
        }

        if !info_parts.is_empty() {
            let info = format!("[ {} ]", info_parts.join(" "));
            write!(f, " {}", info.dimmed())?;
        }

        Ok(())
    }
}

/// Test-only wrapper for displaying providers in selection menus
#[derive(Clone)]
struct CliProvider(AnyProvider);

impl Display for CliProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name_width = ProviderId::built_in_providers()
            .iter()
            .map(|id| id.to_string().len())
            .max()
            .unwrap_or(10);

        let name = self.0.id().to_string();

        match &self.0 {
            AnyProvider::Url(provider) => {
                write!(f, "{} {:<width$}", "✓".green(), name, width = name_width)?;
                if let Some(domain) = provider.url.domain() {
                    write!(f, " [{domain}]")?;
                } else {
                    write!(f, " {}", markers::EMPTY)?;
                }
            }
            AnyProvider::Template(_) => {
                write!(f, "  {name:<name_width$} {}", markers::EMPTY)?;
            }
        }
        Ok(())
    }
}

#[test]
fn test_extract_command_value_with_provided_value() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();
    let command = ForgeCommand {
        name: String::from("/test"),
        description: String::from("Test command"),
        value: None,
    };
    let parts = vec!["arg1", "arg2"];

    // Execute
    let result = cmd_manager.extract_command_value(&command, &parts);

    // Verify
    assert_eq!(result, Some(String::from("arg1 arg2")));
}

#[test]
fn test_extract_command_value_with_empty_parts_default_value() {
    // Setup
    let cmd_manager = ForgeCommandManager {
        commands: Arc::new(Mutex::new(vec![ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: Some(String::from("default_value")),
        }])),
    };
    let command = ForgeCommand {
        name: String::from("/test"),
        description: String::from("Test command"),
        value: None,
    };
    let parts: Vec<&str> = vec![];

    // Execute
    let result = cmd_manager.extract_command_value(&command, &parts);

    // Verify
    assert_eq!(result, Some(String::from("default_value")));
}

#[test]
fn test_extract_command_value_with_empty_string_parts() {
    // Setup
    let cmd_manager = ForgeCommandManager {
        commands: Arc::new(Mutex::new(vec![ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: Some(String::from("default_value")),
        }])),
    };
    let command = ForgeCommand {
        name: String::from("/test"),
        description: String::from("Test command"),
        value: None,
    };
    let parts = vec![""];

    // Execute
    let result = cmd_manager.extract_command_value(&command, &parts);

    // Verify - should use default as the provided value is empty
    assert_eq!(result, Some(String::from("default_value")));
}

#[test]
fn test_extract_command_value_with_whitespace_parts() {
    // Setup
    let cmd_manager = ForgeCommandManager {
        commands: Arc::new(Mutex::new(vec![ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: Some(String::from("default_value")),
        }])),
    };
    let command = ForgeCommand {
        name: String::from("/test"),
        description: String::from("Test command"),
        value: None,
    };
    let parts = vec!["  "];

    // Execute
    let result = cmd_manager.extract_command_value(&command, &parts);

    // Verify - should use default as the provided value is just whitespace
    assert_eq!(result, Some(String::from("default_value")));
}

#[test]
fn test_extract_command_value_no_default_no_provided() {
    // Setup
    let cmd_manager = ForgeCommandManager {
        commands: Arc::new(Mutex::new(vec![ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: None,
        }])),
    };
    let command = ForgeCommand {
        name: String::from("/test"),
        description: String::from("Test command"),
        value: None,
    };
    let parts: Vec<&str> = vec![];

    // Execute
    let result = cmd_manager.extract_command_value(&command, &parts);

    // Verify - should be None as there's no default and no provided value
    assert_eq!(result, None);
}

#[test]
fn test_extract_command_value_provided_overrides_default() {
    // Setup
    let cmd_manager = ForgeCommandManager {
        commands: Arc::new(Mutex::new(vec![ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: Some(String::from("default_value")),
        }])),
    };
    let command = ForgeCommand {
        name: String::from("/test"),
        description: String::from("Test command"),
        value: None,
    };
    let parts = vec!["provided_value"];

    // Execute
    let result = cmd_manager.extract_command_value(&command, &parts);

    // Verify - provided value should override default
    assert_eq!(result, Some(String::from("provided_value")));
}
#[test]
fn test_parse_shell_command() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let result = cmd_manager.parse("!ls -la").unwrap();

    // Verify
    match result {
        AppCommand::Shell(cmd) => assert_eq!(cmd, "ls -la"),
        _ => panic!("Expected Shell command, got {result:?}"),
    }
}

#[test]
fn test_parse_shell_command_empty() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let result = cmd_manager.parse("!").unwrap();

    // Verify
    match result {
        AppCommand::Shell(cmd) => assert_eq!(cmd, ""),
        _ => panic!("Expected Shell command, got {result:?}"),
    }
}

#[test]
fn test_parse_shell_command_with_whitespace() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let result = cmd_manager.parse("!   echo 'test'   ").unwrap();

    // Verify
    match result {
        AppCommand::Shell(cmd) => assert_eq!(cmd, "echo 'test'"),
        _ => panic!("Expected Shell command, got {result:?}"),
    }
}

#[test]
fn test_shell_command_not_in_default_commands() {
    // Setup
    let manager = ForgeCommandManager::default();
    let commands = manager.list();

    // The shell command should not be included
    let contains_shell = commands.iter().any(|cmd| cmd.name == "!shell");
    assert!(
        !contains_shell,
        "Shell command should not be in default commands"
    );
}
#[test]
fn test_parse_list_command() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let result = cmd_manager.parse("/conversation").unwrap();

    // Verify
    match result {
        AppCommand::Conversations { .. } => {
            // Command parsed correctly
        }
        _ => panic!("Expected List command, got {result:?}"),
    }
}

#[test]
fn test_parse_conversation_with_id() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let actual = cmd_manager
        .parse("/conversation 550e8400-e29b-41d4-a716-446655440000")
        .unwrap();

    // Verify
    let expected =
        AppCommand::Conversations { id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()) };
    assert_eq!(actual, expected);
}

#[test]
fn test_list_command_in_default_commands() {
    // Setup
    let manager = ForgeCommandManager::default();
    let commands = manager.list();

    // The list command should be included
    let contains_list = commands.iter().any(|cmd| cmd.name == "conversation");
    assert!(
        contains_list,
        "Conversations command should be in default commands"
    );
}

#[test]
fn test_sanitize_agent_id_basic() {
    // Test basic sanitization
    let fixture = "test-agent";
    let actual = ForgeCommandManager::sanitize_agent_id(fixture);
    let expected = "test-agent";
    assert_eq!(actual, expected);
}

#[test]
fn test_sanitize_agent_id_with_spaces() {
    // Test space replacement
    let fixture = "test agent name";
    let actual = ForgeCommandManager::sanitize_agent_id(fixture);
    let expected = "test-agent-name";
    assert_eq!(actual, expected);
}

#[test]
fn test_sanitize_agent_id_with_special_chars() {
    // Test special character replacement
    let fixture = "test@agent#name!";
    let actual = ForgeCommandManager::sanitize_agent_id(fixture);
    let expected = "test-agent-name";
    assert_eq!(actual, expected);
}

#[test]
fn test_sanitize_agent_id_uppercase() {
    // Test uppercase conversion
    let fixture = "TestAgent";
    let actual = ForgeCommandManager::sanitize_agent_id(fixture);
    let expected = "testagent";
    assert_eq!(actual, expected);
}

#[test]
fn test_is_reserved_command() {
    // Test reserved commands
    assert!(ForgeCommandManager::is_reserved_command("agent"));
    assert!(ForgeCommandManager::is_reserved_command("forge"));
    assert!(ForgeCommandManager::is_reserved_command("muse"));
    assert!(!ForgeCommandManager::is_reserved_command("agent-custom"));
    assert!(!ForgeCommandManager::is_reserved_command("custom"));
}

#[test]
fn test_register_agent_commands() {
    // Setup
    let fixture = ForgeCommandManager::default();
    let agents = vec![
        forge_domain::AgentInfo::default()
            .id("test-agent")
            .title("Test Agent".to_string()),
        forge_domain::AgentInfo::default()
            .id("another")
            .title("Another Agent".to_string()),
    ];

    // Execute
    let result = fixture.register_agent_commands(agents);

    // Verify result
    assert_eq!(result.registered_count, 2);
    assert_eq!(result.skipped_conflicts.len(), 0);

    // Verify
    let commands = fixture.list();
    let agent_commands: Vec<_> = commands
        .iter()
        .filter(|cmd| cmd.name.starts_with("agent-"))
        .collect();

    assert_eq!(agent_commands.len(), 2);
    assert!(
        agent_commands
            .iter()
            .any(|cmd| cmd.name == "agent-test-agent")
    );
    assert!(agent_commands.iter().any(|cmd| cmd.name == "agent-another"));
}

#[test]
fn test_parse_agent_switch_command() {
    // Setup
    let fixture = ForgeCommandManager::default();
    let agents = vec![
        forge_domain::AgentInfo::default()
            .id("test-agent")
            .title("Test Agent".to_string()),
    ];
    let _result = fixture.register_agent_commands(agents);

    // Execute
    let actual = fixture.parse("/agent-test-agent").unwrap();

    // Verify
    match actual {
        AppCommand::AgentSwitch(agent_id) => assert_eq!(agent_id, "test-agent"),
        _ => panic!("Expected AgentSwitch command, got {actual:?}"),
    }
}

fn create_model_fixture(
    id: &str,
    context_length: Option<u64>,
    tools_supported: Option<bool>,
) -> Model {
    Model {
        id: ModelId::new(id),
        name: None,
        description: None,
        context_length,
        tools_supported,
        supports_parallel_tool_calls: None,
        supports_reasoning: None,
        input_modalities: vec![InputModality::Text],
    }
}

#[test]
fn test_cli_model_display_with_context_and_tools() {
    let fixture = create_model_fixture("gpt-4", Some(128000), Some(true));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "gpt-4 [ 128k 🛠️ ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_with_large_context() {
    let fixture = create_model_fixture("claude-3", Some(2000000), Some(true));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "claude-3 [ 2M 🛠️ ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_with_small_context() {
    let fixture = create_model_fixture("small-model", Some(512), Some(false));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "small-model [ 512 ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_with_context_only() {
    let fixture = create_model_fixture("text-model", Some(4096), Some(false));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "text-model [ 4k ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_with_tools_only() {
    let fixture = create_model_fixture("tool-model", None, Some(true));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "tool-model [ 🛠️ ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_empty_context_and_no_tools() {
    let fixture = create_model_fixture("basic-model", None, Some(false));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "basic-model";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_empty_context_and_none_tools() {
    let fixture = create_model_fixture("unknown-model", None, None);
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "unknown-model";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_exact_thousands() {
    let fixture = create_model_fixture("exact-k", Some(8000), Some(true));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "exact-k [ 8k 🛠️ ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_exact_millions() {
    let fixture = create_model_fixture("exact-m", Some(1000000), Some(true));
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "exact-m [ 1M 🛠️ ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_edge_case_999() {
    let fixture = create_model_fixture("edge-999", Some(999), None);
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "edge-999 [ 999 ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_model_display_edge_case_1001() {
    let fixture = create_model_fixture("edge-1001", Some(1001), None);
    let formatted = format!("{}", CliModel(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "edge-1001 [ 1k ]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_provider_display_minimal() {
    let fixture = AnyProvider::Url(Provider {
        id: ProviderId::OPENAI,
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Url::parse("https://api.openai.com/v1/chat/completions").unwrap(),
        auth_methods: vec![forge_domain::AuthMethod::ApiKey],
        url_params: vec![],
        credential: None,
        custom_headers: None,
        models: Some(ModelSource::Url(
            Url::parse("https://api.openai.com/v1/models").unwrap(),
        )),
    });
    let formatted = format!("{}", CliProvider(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "✓ OpenAI                    [api.openai.com]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_provider_display_with_subdomain() {
    let fixture = AnyProvider::Url(Provider {
        id: ProviderId::OPEN_ROUTER,
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Url::parse("https://openrouter.ai/api/v1/chat/completions").unwrap(),
        auth_methods: vec![forge_domain::AuthMethod::ApiKey],
        url_params: vec![],
        credential: None,
        custom_headers: None,
        models: Some(ModelSource::Url(
            Url::parse("https://openrouter.ai/api/v1/models").unwrap(),
        )),
    });
    let formatted = format!("{}", CliProvider(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "✓ OpenRouter                [openrouter.ai]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_provider_display_no_domain() {
    let fixture = AnyProvider::Url(Provider {
        id: ProviderId::FORGE,
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Url::parse("http://localhost:8080/chat/completions").unwrap(),
        auth_methods: vec![forge_domain::AuthMethod::ApiKey],
        url_params: vec![],
        credential: None,
        custom_headers: None,
        models: Some(ModelSource::Url(
            Url::parse("http://localhost:8080/models").unwrap(),
        )),
    });
    let formatted = format!("{}", CliProvider(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = "✓ Forge                     [localhost]";
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_provider_display_template() {
    let fixture = AnyProvider::Template(Provider {
        id: ProviderId::ANTHROPIC,
        provider_type: Default::default(),
        response: Some(ProviderResponse::Anthropic),
        url: Template::new("https://api.anthropic.com/v1/messages"),
        auth_methods: vec![forge_domain::AuthMethod::ApiKey],
        url_params: vec![],
        credential: None,
        custom_headers: None,
        models: Some(ModelSource::Url(Template::new(
            "https://api.anthropic.com/v1/models",
        ))),
    });
    let formatted = format!("{}", CliProvider(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = format!("  Anthropic                 {}", markers::EMPTY);
    assert_eq!(actual, expected);
}

#[test]
fn test_cli_provider_display_ip_address() {
    let fixture = AnyProvider::Url(Provider {
        id: ProviderId::FORGE,
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Url::parse("http://192.168.1.1:8080/chat/completions").unwrap(),
        auth_methods: vec![forge_domain::AuthMethod::ApiKey],
        url_params: vec![],
        credential: None,
        custom_headers: None,
        models: Some(ModelSource::Url(
            Url::parse("http://192.168.1.1:8080/models").unwrap(),
        )),
    });
    let formatted = format!("{}", CliProvider(fixture));
    let actual = strip_ansi_codes(&formatted);
    let expected = format!("✓ Forge                     {}", markers::EMPTY);
    assert_eq!(actual, expected);
}

#[test]
fn test_parse_commit_command() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/commit").unwrap();
    match actual {
        AppCommand::Commit { max_diff_size, .. } => {
            assert_eq!(max_diff_size, None);
        }
        _ => panic!("Expected Commit command, got {actual:?}"),
    }
}

#[test]
fn test_parse_commit_command_with_preview() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/commit preview").unwrap();
    match actual {
        AppCommand::Commit { max_diff_size, .. } => {
            assert_eq!(max_diff_size, None);
        }
        _ => panic!("Expected Commit command with preview, got {actual:?}"),
    }
}

#[test]
fn test_parse_commit_command_with_max_diff() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/commit 5000").unwrap();
    match actual {
        AppCommand::Commit { max_diff_size, .. } => {
            assert_eq!(max_diff_size, Some(5000));
        }
        _ => panic!("Expected Commit command with max_diff_size, got {actual:?}"),
    }
}

#[test]
fn test_parse_commit_command_with_all_flags() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/commit preview 10000").unwrap();
    match actual {
        AppCommand::Commit { max_diff_size, .. } => {
            assert_eq!(max_diff_size, Some(10000));
        }
        _ => panic!("Expected Commit command with all flags, got {actual:?}"),
    }
}

#[test]
fn test_commit_command_in_default_commands() {
    let manager = ForgeCommandManager::default();
    let commands = manager.list();
    let contains_commit = commands.iter().any(|cmd| cmd.name == "commit");
    assert!(
        contains_commit,
        "Commit command should be in default commands"
    );
}

#[test]
fn test_parse_invalid_agent_command() {
    // Setup
    let fixture = ForgeCommandManager::default();

    // Execute
    let result = fixture.parse("/agent-nonexistent");

    // Verify
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not a valid agent command")
    );
}

#[test]
fn test_parse_invalid_command_with_colon_returns_helpful_error() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse(":celar").unwrap_err().to_string();
    let expected = "Unknown command ':celar'. Run ':help' to list available commands.".to_string();
    assert_eq!(actual, expected);
}

#[test]
fn test_parse_invalid_command_with_slash_returns_helpful_error() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/celar").unwrap_err().to_string();
    let expected = "Unknown command '/celar'. Run '/help' to list available commands.".to_string();
    assert_eq!(actual, expected);
}

#[test]
fn test_parse_tool_command() {
    // Setup
    let fixture = ForgeCommandManager::default();

    // Execute
    let result = fixture.parse("/tools").unwrap();

    // Verify
    match result {
        AppCommand::Tools => {
            // Command parsed correctly
        }
        _ => panic!("Expected Tool command, got {result:?}"),
    }
}

#[test]
fn test_parse_dump_command_json() {
    // Setup
    let fixture = ForgeCommandManager::default();

    // Execute
    let actual = fixture.parse("/dump").unwrap();

    // Verify
    let expected = AppCommand::Dump { html: false };
    assert_eq!(actual, expected);
}

#[test]
fn test_parse_dump_command_html_with_flag() {
    // Setup
    let fixture = ForgeCommandManager::default();

    // Execute
    let actual = fixture.parse("/dump --html").unwrap();

    // Verify
    let expected = AppCommand::Dump { html: true };
    assert_eq!(actual, expected);
}

#[test]
fn test_parse_rename_command() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/rename my-session").unwrap();
    assert_eq!(
        actual,
        AppCommand::Rename { name: vec!["my-session".to_string()] }
    );
}

#[test]
fn test_parse_rename_command_multi_word() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/rename auth refactor work").unwrap();
    assert_eq!(
        actual,
        AppCommand::Rename {
            name: vec![
                "auth".to_string(),
                "refactor".to_string(),
                "work".to_string()
            ]
        }
    );
}

#[test]
fn test_parse_rename_command_no_name() {
    let fixture = ForgeCommandManager::default();
    let result = fixture.parse("/rename");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("provide a name"));
}

#[test]
fn test_parse_rename_alias() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/rn my-session").unwrap();
    assert_eq!(
        actual,
        AppCommand::Rename { name: vec!["my-session".to_string()] }
    );
}

#[test]
fn test_parse_rename_trims_whitespace() {
    let fixture = ForgeCommandManager::default();
    let actual = fixture.parse("/rename   my title   ").unwrap();
    assert_eq!(
        actual,
        AppCommand::Rename { name: vec!["my".to_string(), "title".to_string()] }
    );
}

#[test]
fn test_rename_is_reserved_command() {
    assert!(ForgeCommandManager::is_reserved_command("rename"));
    assert!(ForgeCommandManager::is_reserved_command("rn"));
}

#[test]
fn test_rename_command_name() {
    let cmd = AppCommand::Rename { name: vec!["test".to_string()] };
    assert_eq!(cmd.name(), "rename");
}

#[test]
fn test_parse_suggest_with_dash_prefixed_tokens() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let result = cmd_manager.parse(":suggest --- date").unwrap();

    // Verify
    assert_eq!(
        result,
        AppCommand::Suggest { description: vec!["---".to_string(), "date".to_string()] }
    );
}

#[test]
fn test_parse_suggest_with_double_dash_flags() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let result = cmd_manager.parse(":suggest --date tomorrow").unwrap();

    // Verify
    assert_eq!(
        result,
        AppCommand::Suggest {
            description: vec!["--date".to_string(), "tomorrow".to_string()]
        }
    );
}

#[test]
fn test_parse_suggest_with_single_dash() {
    // Setup
    let cmd_manager = ForgeCommandManager::default();

    // Execute
    let result = cmd_manager.parse(":suggest -v file.txt").unwrap();

    // Verify
    assert_eq!(
        result,
        AppCommand::Suggest { description: vec!["-v".to_string(), "file.txt".to_string()] }
    );
}

use std::path::PathBuf;
use forge_api::Environment;
use forge_domain::EventValue;

// Helper to create minimal test environment
fn create_env(os: &str, home: Option<&str>) -> Environment {
    use fake::{Fake, Faker};
    let mut fixture: Environment = Faker.fake();
    fixture = fixture.os(os.to_string());
    if let Some(home_path) = home {
        fixture = fixture.home(PathBuf::from(home_path));
    }
    fixture
}

#[test]
fn test_format_path_for_display_unix_home() {
    let fixture = create_env("linux", Some("/home/user"));
    let path = PathBuf::from("/home/user/project");

    let actual = super::format_path_for_display(&fixture, &path);
    let expected = "~/project";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_path_for_display_windows_home() {
    let fixture = create_env("windows", Some("C:\\Users\\User"));
    let path = PathBuf::from("C:\\Users\\User\\project");

    let actual = super::format_path_for_display(&fixture, &path);
    let expected = "C:\\Users\\User\\project";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_path_for_display_windows_home_with_spaces() {
    let fixture = create_env("windows", Some("C:\\Users\\User Name"));
    let path = PathBuf::from("C:\\Users\\User Name\\project");

    let actual = super::format_path_for_display(&fixture, &path);
    let expected = "\"C:\\Users\\User Name\\project\"";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_path_for_display_absolute() {
    let fixture = create_env("linux", Some("/home/user"));
    let path = PathBuf::from("/var/log/app");

    let actual = super::format_path_for_display(&fixture, &path);
    let expected = "/var/log/app";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_path_for_display_absolute_windows_with_spaces() {
    let fixture = create_env("windows", Some("C:/Users/User"));
    let path = PathBuf::from("C:/Program Files/App");

    let actual = super::format_path_for_display(&fixture, &path);
    let expected = "\"C:/Program Files/App\"";
    assert_eq!(actual, expected);
}

#[test]
fn test_create_progress_bar() {
    // Test normal case - 70% of 20 = 14 filled, 6 empty
    let actual = super::create_progress_bar(70, 100, 20);
    let expected = "▐██████████████░░░░░░ 70.0%";
    assert_eq!(actual, expected);

    // Test 100% case
    let actual = super::create_progress_bar(100, 100, 20);
    let expected = "▐████████████████████ 100.0%";
    assert_eq!(actual, expected);

    // Test 0% case
    let actual = super::create_progress_bar(0, 100, 20);
    let expected = "▐░░░░░░░░░░░░░░░░░░░░ 0.0%";
    assert_eq!(actual, expected);

    // Test zero limit case
    let actual = super::create_progress_bar(50, 0, 20);
    let expected = "N/A";
    assert_eq!(actual, expected);

    // Test over 100% case (should cap at 100%)
    let actual = super::create_progress_bar(150, 100, 20);
    let expected = "▐████████████████████ 100.0%";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_path_for_display_no_home() {
    let fixture = create_env("linux", None);
    let path = PathBuf::from("/home/user/project");

    let actual = super::format_path_for_display(&fixture, &path);
    let expected = "/home/user/project";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_reset_time_hours_and_minutes() {
    let actual = super::format_reset_time(3661); // 1 hour, 1 minute, 1 second
    let expected = "1h 1m 1s";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_reset_time_hours_only() {
    let actual = super::format_reset_time(3600); // exactly 1 hour
    let expected = "1h";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_reset_time_minutes_and_seconds() {
    let actual = super::format_reset_time(125); // 2 minutes, 5 seconds
    let expected = "2m 5s";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_reset_time_minutes_only() {
    let actual = super::format_reset_time(120); // exactly 2 minutes
    let expected = "2m";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_reset_time_seconds_only() {
    let actual = super::format_reset_time(45); // 45 seconds
    let expected = "45s";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_reset_time_zero() {
    let actual = super::format_reset_time(0);
    let expected = "now";
    assert_eq!(actual, expected);
}

#[test]
fn test_format_reset_time_large_value() {
    let actual = super::format_reset_time(7265); // 2 hours, 1 minute, 5 seconds
    let expected = "2h 1m 5s";
    assert_eq!(actual, expected);
}
#[test]
fn test_metrics_info_display() {
    use forge_api::Metrics;
    use forge_domain::{FileOperation, ToolKind};

    let fixture = Metrics::default()
        .started_at(chrono::Utc::now())
        .insert(
            "src/main.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(12u64)
                .lines_removed(3u64),
        )
        .insert(
            "src/agent/mod.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(8u64)
                .lines_removed(2u64),
        )
        .insert(
            "tests/integration/test_agent.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(5u64)
                .lines_removed(0u64),
        );

    let actual = super::Info::from(&fixture);
    let expected_display = actual.to_string();

    // Verify it contains the task completed section
    assert!(expected_display.contains("TASK COMPLETED"));

    // Verify it contains the files as keys with colons
    assert!(expected_display.contains("main.rs"));
    assert!(expected_display.contains("−3 +12"));
    assert!(expected_display.contains("mod.rs"));
    assert!(expected_display.contains("−2 +8"));
    assert!(expected_display.contains("test_agent.rs"));
    assert!(expected_display.contains("0 +5"));
}

#[test]
fn test_conversation_info_display() {
    use chrono::Utc;
    use forge_api::ConversationId;
    use forge_domain::{FileOperation, ToolKind};

    use super::{Conversation, Metrics};

    let conversation_id = ConversationId::generate();
    let metrics = Metrics::default()
        .started_at(Utc::now())
        .insert(
            "src/main.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(5u64)
                .lines_removed(2u64),
        )
        .insert(
            "tests/test.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(3u64)
                .lines_removed(1u64),
        );

    let fixture = Conversation {
        id: conversation_id,
        title: Some("Test Conversation".to_string()),
        context: None,
        metrics,
        metadata: forge_domain::MetaData::new(Utc::now()),
    };

    let actual = super::Info::from(&fixture);
    let expected_display = actual.to_string();

    // Verify it contains the conversation section
    assert!(expected_display.contains("CONVERSATION"));
    assert!(expected_display.contains("Test Conversation"));
    assert!(expected_display.contains(&conversation_id.to_string()));
}

#[test]
fn test_conversation_info_display_untitled() {
    use chrono::Utc;
    use forge_api::ConversationId;

    use super::{Conversation, Metrics};

    let conversation_id = ConversationId::generate();
    let metrics = Metrics::default().started_at(Utc::now());

    let fixture = Conversation {
        id: conversation_id,
        title: None,
        context: None,
        metrics,
        metadata: forge_domain::MetaData::new(Utc::now()),
    };

    let actual = super::Info::from(&fixture);
    let expected_display = actual.to_string();

    // Verify it contains the conversation section with untitled
    assert!(expected_display.contains("CONVERSATION"));
    assert!(!expected_display.contains("Title:"));
    assert!(expected_display.contains(&conversation_id.to_string()));
}

#[test]
fn test_conversation_info_display_with_task() {
    use chrono::Utc;
    use forge_api::{Context, ContextMessage, ConversationId, Role};

    use super::{Conversation, Metrics};

    let conversation_id = ConversationId::generate();
    let metrics = Metrics::default().started_at(Utc::now());

    // Create a context with user messages
    let context = Context::default()
        .add_message(ContextMessage::system("System prompt"))
        .add_message(ContextMessage::Text(
            forge_domain::TextMessage::new(Role::User, "First user message")
                .raw_content(EventValue::text("First user message")),
        ))
        .add_message(ContextMessage::assistant(
            "Assistant response",
            None,
            None,
            None,
        ))
        .add_message(ContextMessage::Text(
            forge_domain::TextMessage::new(Role::User, "Create a new feature")
                .raw_content(EventValue::text("Create a new feature")),
        ));

    let fixture = Conversation {
        id: conversation_id,
        title: Some("Test Task".to_string()),
        context: Some(context),
        metrics,
        metadata: forge_domain::MetaData::new(Utc::now()),
    };

    let actual = super::Info::from(&fixture);
    let expected_display = actual.to_string();

    // Verify it contains the conversation section with task
    assert!(expected_display.contains("CONVERSATION"));
    assert!(expected_display.contains("Test Task"));
    // Check for Task separately due to ANSI color codes
    assert!(expected_display.contains("Task"));
    assert!(expected_display.contains("Create a new feature"));
    assert!(expected_display.contains(&conversation_id.to_string()));
}

#[test]
fn test_info_display_with_consistent_key_padding() {
    use super::Info;

    let fixture = Info::new()
        .add_title("SECTION ONE")
        .add_key_value("Short", "value1")
        .add_key_value("Very Long Key", "value2")
        .add_key_value("Mid", "value3")
        .add_title("SECTION TWO")
        .add_key_value("A", "valueA")
        .add_key_value("ABC", "valueB");

    let actual = fixture.to_string();

    // Strip ANSI codes for easier assertion
    let stripped = strip_ansi_escapes::strip(&actual);
    let actual_str = String::from_utf8(stripped).unwrap();

    // Verify that keys are padded within each section
    // In SECTION ONE, all keys should be padded to length of "Very Long Key" (13)
    // In SECTION TWO, all keys should be padded to length of "ABC" (3)

    // Check that the display contains properly formatted sections
    assert!(actual_str.contains("SECTION ONE"));
    assert!(actual_str.contains("SECTION TWO"));

    // Verify padding by checking alignment of values
    // All keys in a section should have values starting at the same column
    let lines: Vec<&str> = actual_str.lines().collect();

    // Find SECTION ONE items
    let section_one_start = lines
        .iter()
        .position(|l| l.contains("SECTION ONE"))
        .unwrap();
    let section_two_start = lines
        .iter()
        .position(|l| l.contains("SECTION TWO"))
        .unwrap();

    let section_one_items: Vec<&str> = lines[section_one_start + 1..section_two_start]
        .iter()
        .filter(|l| !l.trim().is_empty() && !l.contains("SECTION"))
        .copied()
        .collect();

    // All values in section one should start at the same position
    // Find where "value" starts in each line
    let value_positions: Vec<usize> = section_one_items
        .iter()
        .map(|line| line.find("value").unwrap())
        .collect();

    assert!(
        value_positions.windows(2).all(|w| w[0] == w[1]),
        "Values in SECTION ONE should be aligned. Value positions: {:?}",
        value_positions
    );

    // Check SECTION TWO items
    let section_two_items: Vec<&str> = lines[section_two_start + 1..]
        .iter()
        .filter(|l| !l.trim().is_empty() && !l.contains("SECTION"))
        .copied()
        .collect();

    let value_positions_two: Vec<usize> = section_two_items
        .iter()
        .map(|line| line.find("value").unwrap())
        .collect();

    assert!(
        value_positions_two.windows(2).all(|w| w[0] == w[1]),
        "Values in SECTION TWO should be aligned. Value positions: {:?}",
        value_positions_two
    );

    // Verify that different sections can have different padding
    // (SECTION ONE should have wider padding than SECTION TWO)
    assert!(
        value_positions[0] > value_positions_two[0],
        "SECTION ONE should have wider padding than SECTION TWO"
    );
}

#[test]
fn test_add_key_value_normalizes_to_lowercase() {
    let info = super::Info::new()
        .add_key_value("VERSION", "1.0.0")
        .add_key_value("Working Directory", "/home/user")
        .add_key_value("Mixed CASE Key", "value");

    let display = info.to_string();

    // All keys should be lowercase - checking just the key part without exact
    // formatting
    assert!(display.contains("version"));
    assert!(display.contains("working directory"));
    assert!(display.contains("mixed case key"));

    // Values should be preserved
    assert!(display.contains("1.0.0"));
    assert!(display.contains("/home/user"));
    assert!(display.contains("value"));

    // Should not contain uppercase versions
    assert!(!display.contains("VERSION"));
    assert!(!display.contains("Working Directory"));
    assert!(!display.contains("Mixed CASE Key"));
}

#[test]
fn test_info_from_command_manager() {
    let command_manager = super::ForgeCommandManager::default();
    let info = super::Info::from(&command_manager);
    let display = info.to_string();

    // Verify compile-time detection works correctly
    #[cfg(target_os = "macos")]
    {
        assert!(display.contains("<opt+enter>"));
        assert!(!display.contains("<alt+enter>"));
    }

    #[cfg(not(target_os = "macos"))]
    assert!(display.contains("<shift+enter>"));


    // Should contain standard sections
    assert!(display.contains("COMMANDS"));
    assert!(display.contains("KEYBOARD SHORTCUTS"));
    assert!(display.contains("<ctrl+c>"));
    assert!(display.contains("<ctrl+d>"));
}

#[test]
fn test_metrics_info_filters_zero_changes() {
    use forge_api::Metrics;
    use forge_domain::{FileOperation, ToolKind};

    let fixture = Metrics::default()
        .started_at(chrono::Utc::now())
        .insert(
            "src/main.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(12u64)
                .lines_removed(3u64),
        )
        .insert(
            "src/no_changes.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(0u64)
                .lines_removed(0u64),
        )
        .insert(
            "src/agent/mod.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(8u64)
                .lines_removed(2u64),
        );

    let actual = super::Info::from(&fixture);
    let expected_display = actual.to_string();

    // Verify it contains the task completed section
    assert!(expected_display.contains("TASK COMPLETED"));

    // Verify it contains files with changes as keys
    assert!(expected_display.contains("main.rs"));
    assert!(expected_display.contains("−3 +12"));
    assert!(expected_display.contains("mod.rs"));
    assert!(expected_display.contains("−2 +8"));

    // Verify it does NOT contain the file with zero changes
    assert!(!expected_display.contains("no_changes.rs"));
}

#[test]
fn test_metrics_info_all_zero_changes_shows_no_changes() {
    use forge_api::Metrics;
    use forge_domain::{FileOperation, ToolKind};

    let fixture = Metrics::default()
        .started_at(chrono::Utc::now())
        .insert(
            "src/file1.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(0u64)
                .lines_removed(0u64),
        )
        .insert(
            "src/file2.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(0u64)
                .lines_removed(0u64),
        );

    let actual = super::Info::from(&fixture);
    let expected_display = actual.to_string();

    // Verify it shows "No Changes Produced" when all files have zero changes
    assert!(expected_display.contains("[No Changes Produced]"));
    assert!(!expected_display.contains("file1.rs"));
    assert!(!expected_display.contains("file2.rs"));
}

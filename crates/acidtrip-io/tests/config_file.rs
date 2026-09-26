use std::sync::Mutex;

use acidtrip_io::backup::BackupMode;
use acidtrip_io::config::{Config, DEFAULT_CONFIG_TOML};

static ENV: Mutex<()> = Mutex::new(());

#[test]
fn default_file_is_created_and_reparses_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("cfg/config.toml");
    let c = Config::load_or_create(&path).unwrap();
    assert_eq!(c, Config::default());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, DEFAULT_CONFIG_TOML);
    assert!(text.contains("ANTHROPIC_API_KEY") && text.contains("\"numbered\"") && text.contains("\"acid\""));
    assert_eq!(Config::load_or_create(&path).unwrap(), Config::default());
    assert_eq!(Config::parse(DEFAULT_CONFIG_TOML).unwrap(), Config::default());
}

#[test]
fn defaults() {
    let c = Config::default();
    assert_eq!(c.autosave_seconds, 20);
    assert_eq!(c.version_every_minutes, 5);
    assert_eq!(c.backup, BackupMode::Bak);
    assert_eq!(c.keymap.preset, "modern");
    assert_eq!((c.new_doc.width, c.new_doc.height), (80, 25));
}

#[test]
fn overrides_and_unknown_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
backup = "numbered"
autosave_seconds = 5
mystery = 1
[keymap]
preset = "acid"
[keymap.bindings]
save = ["ctrl-s", "f2"]
[ai]
model = "claude-opus-5"
whatever = "x"
[unknown_table]
a = 1
"#,
    )
    .unwrap();
    let c = Config::load_or_create(&path).unwrap();
    assert_eq!(c.backup, BackupMode::Numbered);
    assert_eq!(c.autosave_seconds, 5);
    assert_eq!(c.keymap.preset, "acid");
    assert_eq!(c.keymap.bindings["save"], vec!["ctrl-s", "f2"]);
    assert_eq!(c.ai.model, "claude-opus-5");
    assert_eq!(c.ai.max_tool_rounds, 24);
    assert_eq!(c.ui, Config::default().ui);
}

#[test]
fn bad_toml_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    std::fs::write(&path, "backup = \n[[[").unwrap();
    assert!(Config::load_or_create(&path).is_err());
    std::fs::write(&path, "backup = \"sometimes\"").unwrap();
    assert!(Config::load_or_create(&path).is_err());
    // A bad file is never overwritten.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "backup = \"sometimes\"");
}

#[test]
fn api_key_precedence() {
    let _g = ENV.lock().unwrap();
    let mut c = Config::default();
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
    assert_eq!(c.api_key(), None);
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "") };
    assert_eq!(c.api_key(), None);
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "env-key") };
    assert_eq!(c.api_key().as_deref(), Some("env-key"));
    c.ai.api_key = "cfg-key".into();
    assert_eq!(c.api_key().as_deref(), Some("cfg-key"));
    c.ai.api_key = "  ".into();
    assert_eq!(c.api_key().as_deref(), Some("env-key"));
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
}

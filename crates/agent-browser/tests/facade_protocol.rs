use agent_browser::{BrowserOptions, BrowserSession};

#[test]
fn browser_session_builds_navigation_command_for_open_url() {
    let session = BrowserSession::new(BrowserOptions {
        session: "reshape-main".to_string(),
        allow_file_access: true,
        ..BrowserOptions::default()
    });

    let command = session.open_command("file:///tmp/index.html");

    assert_eq!(command["action"], "navigate");
    assert_eq!(command["url"], "file:///tmp/index.html");
    assert!(command["id"].as_str().unwrap().starts_with('r'));
}

#[test]
fn browser_session_builds_core_rendering_commands() {
    let session = BrowserSession::new(BrowserOptions::default());

    assert_eq!(session.reload_command()["action"], "reload");
    assert_eq!(session.snapshot_command()["action"], "snapshot");

    let screenshot = session.screenshot_command(Some("out.png"));
    assert_eq!(screenshot["action"], "screenshot");
    assert_eq!(screenshot["path"], "out.png");
    assert_eq!(screenshot["selector"], serde_json::Value::Null);

    assert_eq!(session.close_command()["action"], "close");
}

#[test]
fn browser_options_default_has_no_extensions() {
    assert!(BrowserOptions::default().extensions.is_empty());
}

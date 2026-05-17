use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::commands::gen_id;
use crate::connection::{cleanup_stale_files, send_command};

pub type BrowserResponse = crate::connection::Response;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserOptions {
    pub session: String,
    pub headed: bool,
    pub debug: bool,
    pub allow_file_access: bool,
    pub extensions: Vec<String>,
    pub idle_timeout: Option<String>,
    pub default_timeout: Option<u64>,
    pub no_auto_dialog: bool,
}

impl Default for BrowserOptions {
    fn default() -> Self {
        Self {
            session: "default".to_string(),
            headed: false,
            debug: false,
            allow_file_access: false,
            extensions: Vec::new(),
            idle_timeout: None,
            default_timeout: None,
            no_auto_dialog: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSession {
    options: BrowserOptions,
}

impl BrowserSession {
    pub fn new(options: BrowserOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &BrowserOptions {
        &self.options
    }

    pub fn ensure(&self) -> Result<(), BrowserError> {
        if self.daemon_ready() {
            return Ok(());
        }

        cleanup_stale_files(&self.options.session);
        self.apply_daemon_environment();

        let session = self.options.session.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("failed to create daemon runtime");
            runtime.block_on(crate::native::daemon::run_daemon(&session));
        });

        for _ in 0..25 {
            thread::sleep(Duration::from_millis(200));
            if self.daemon_ready() {
                return Ok(());
            }
        }

        Err(BrowserError::DaemonStart {
            session: self.options.session.clone(),
        })
    }

    pub fn send(&self, command: Value) -> Result<BrowserResponse, BrowserError> {
        send_command(command, &self.options.session).map_err(BrowserError::Transport)
    }

    pub fn open(&self, url: impl AsRef<str>) -> Result<BrowserResponse, BrowserError> {
        self.ensure()?;
        self.send(self.open_command(url))
    }

    pub fn reload(&self) -> Result<BrowserResponse, BrowserError> {
        self.ensure()?;
        self.send(self.reload_command())
    }

    pub fn snapshot(&self) -> Result<BrowserResponse, BrowserError> {
        self.ensure()?;
        self.send(self.snapshot_command())
    }

    pub fn screenshot(
        &self,
        path: Option<impl AsRef<str>>,
    ) -> Result<BrowserResponse, BrowserError> {
        self.ensure()?;
        self.send(self.screenshot_command(path))
    }

    pub fn close(&self) -> Result<BrowserResponse, BrowserError> {
        self.send(self.close_command())
    }

    pub fn open_command(&self, url: impl AsRef<str>) -> Value {
        json!({
            "id": gen_id(),
            "action": "navigate",
            "url": normalize_url(url.as_ref()),
        })
    }

    pub fn reload_command(&self) -> Value {
        json!({ "id": gen_id(), "action": "reload" })
    }

    pub fn snapshot_command(&self) -> Value {
        json!({ "id": gen_id(), "action": "snapshot" })
    }

    pub fn screenshot_command(&self, path: Option<impl AsRef<str>>) -> Value {
        json!({
            "id": gen_id(),
            "action": "screenshot",
            "path": path.map(|p| p.as_ref().to_string()),
            "selector": Value::Null,
        })
    }

    pub fn close_command(&self) -> Value {
        json!({ "id": gen_id(), "action": "close" })
    }

    fn daemon_ready(&self) -> bool {
        let command = json!({ "id": gen_id(), "action": "stream_status" });
        send_command(command, &self.options.session).is_ok()
    }

    fn apply_daemon_environment(&self) {
        std::env::set_var("AGENT_BROWSER_SESSION", &self.options.session);
        std::env::set_var("AGENT_BROWSER_EMBEDDED_DAEMON", "1");
        if self.options.headed {
            std::env::set_var("AGENT_BROWSER_HEADED", "1");
        }
        if self.options.debug {
            std::env::set_var("AGENT_BROWSER_DEBUG", "1");
        }
        if self.options.allow_file_access {
            std::env::set_var("AGENT_BROWSER_ALLOW_FILE_ACCESS", "1");
        }
        if !self.options.extensions.is_empty() {
            std::env::set_var(
                "AGENT_BROWSER_EXTENSIONS",
                self.options.extensions.join(","),
            );
        }
        if let Some(idle_timeout) = &self.options.idle_timeout {
            std::env::set_var("AGENT_BROWSER_IDLE_TIMEOUT_MS", idle_timeout);
        }
        if let Some(default_timeout) = self.options.default_timeout {
            std::env::set_var("AGENT_BROWSER_DEFAULT_TIMEOUT", default_timeout.to_string());
        }
        if self.options.no_auto_dialog {
            std::env::set_var("AGENT_BROWSER_NO_AUTO_DIALOG", "1");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserError {
    DaemonStart { session: String },
    Transport(String),
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrowserError::DaemonStart { session } => {
                write!(
                    f,
                    "failed to start agent-browser daemon for session {session}"
                )
            }
            BrowserError::Transport(error) => write!(f, "agent-browser transport failed: {error}"),
        }
    }
}

impl std::error::Error for BrowserError {}

fn normalize_url(url: &str) -> String {
    let lower = url.to_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("about:")
        || lower.starts_with("data:")
        || lower.starts_with("file:")
        || lower.starts_with("chrome-extension://")
        || lower.starts_with("chrome://")
    {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

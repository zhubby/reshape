use std::sync::{Arc, Mutex};

use reshape_browser::{AgentBrowserRenderer, BrowserRenderer, BrowserSessionClient};

#[test]
fn workspace_entry_url_uses_canonical_file_url() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("index.html");
    std::fs::write(&entry, "<h1>Hello</h1>").unwrap();

    let url = AgentBrowserRenderer::<FakeBrowserSession>::workspace_entry_url(&entry).unwrap();

    assert_eq!(url.scheme(), "file");
    assert!(url.as_str().ends_with("/index.html"));
}

#[test]
fn workspace_entry_url_rejects_directories() {
    let dir = tempfile::tempdir().unwrap();

    let error = AgentBrowserRenderer::<FakeBrowserSession>::workspace_entry_url(dir.path())
        .expect_err("directory should not be a renderable workspace entry");

    assert!(matches!(
        error,
        reshape_browser::BrowserRenderError::NotAFile(_)
    ));
}

#[test]
fn open_workspace_entry_sends_file_url_to_browser_session() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("index.html");
    std::fs::write(&entry, "<h1>Hello</h1>").unwrap();
    let fake = FakeBrowserSession::default();
    let calls = fake.calls.clone();
    let renderer = AgentBrowserRenderer::new(fake);

    renderer.open_workspace_entry(&entry).unwrap();

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("file://"));
    assert!(calls[0].ends_with("/index.html"));
}

#[derive(Default)]
struct FakeBrowserSession {
    calls: Arc<Mutex<Vec<String>>>,
}

impl BrowserSessionClient for FakeBrowserSession {
    fn open(&self, url: &str) -> Result<(), reshape_browser::BrowserRenderError> {
        self.calls.lock().unwrap().push(url.to_string());
        Ok(())
    }

    fn reload(&self) -> Result<(), reshape_browser::BrowserRenderError> {
        Ok(())
    }

    fn snapshot(&self) -> Result<(), reshape_browser::BrowserRenderError> {
        Ok(())
    }

    fn screenshot(&self, _path: Option<&str>) -> Result<(), reshape_browser::BrowserRenderError> {
        Ok(())
    }

    fn close(&self) -> Result<(), reshape_browser::BrowserRenderError> {
        Ok(())
    }
}

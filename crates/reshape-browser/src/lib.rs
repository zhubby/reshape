use std::path::{Path, PathBuf};

use ::agent_browser::{BrowserOptions, BrowserSession};
use url::Url;

pub mod agent_browser;

pub type Result<T> = std::result::Result<T, BrowserRenderError>;

pub trait BrowserRenderer {
    fn open_workspace_entry(&self, path: &Path) -> Result<()>;
    fn reload(&self) -> Result<()>;
    fn snapshot(&self) -> Result<()>;
    fn screenshot(&self, path: Option<&Path>) -> Result<()>;
    fn close(&self) -> Result<()>;
}

pub trait BrowserSessionClient {
    fn open(&self, url: &str) -> Result<()>;
    fn reload(&self) -> Result<()>;
    fn snapshot(&self) -> Result<()>;
    fn screenshot(&self, path: Option<&str>) -> Result<()>;
    fn close(&self) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct AgentBrowserRenderer<C = BrowserSession> {
    session: C,
}

impl<C> AgentBrowserRenderer<C> {
    pub fn new(session: C) -> Self {
        Self { session }
    }

    pub fn workspace_entry_url(path: &Path) -> Result<Url> {
        let metadata = std::fs::metadata(path).map_err(|source| BrowserRenderError::FileUrl {
            path: path.to_path_buf(),
            reason: Some(source.to_string()),
        })?;
        if !metadata.is_file() {
            return Err(BrowserRenderError::NotAFile(path.to_path_buf()));
        }

        let canonical = path
            .canonicalize()
            .map_err(|source| BrowserRenderError::FileUrl {
                path: path.to_path_buf(),
                reason: Some(source.to_string()),
            })?;

        Url::from_file_path(&canonical).map_err(|_| BrowserRenderError::FileUrl {
            path: canonical,
            reason: None,
        })
    }
}

impl Default for AgentBrowserRenderer<BrowserSession> {
    fn default() -> Self {
        let options = BrowserOptions {
            session: "reshape-main".to_string(),
            allow_file_access: true,
            ..BrowserOptions::default()
        };
        Self::new(BrowserSession::new(options))
    }
}

impl<C> BrowserRenderer for AgentBrowserRenderer<C>
where
    C: BrowserSessionClient,
{
    fn open_workspace_entry(&self, path: &Path) -> Result<()> {
        let url = Self::workspace_entry_url(path)?;
        self.session.open(url.as_str())
    }

    fn reload(&self) -> Result<()> {
        self.session.reload()
    }

    fn snapshot(&self) -> Result<()> {
        self.session.snapshot()
    }

    fn screenshot(&self, path: Option<&Path>) -> Result<()> {
        let path = path.map(|path| path.to_string_lossy().to_string());
        self.session.screenshot(path.as_deref())
    }

    fn close(&self) -> Result<()> {
        self.session.close()
    }
}

impl BrowserSessionClient for BrowserSession {
    fn open(&self, url: &str) -> Result<()> {
        self.open(url).map(|_| ()).map_err(BrowserRenderError::from)
    }

    fn reload(&self) -> Result<()> {
        self.reload().map(|_| ()).map_err(BrowserRenderError::from)
    }

    fn snapshot(&self) -> Result<()> {
        self.snapshot()
            .map(|_| ())
            .map_err(BrowserRenderError::from)
    }

    fn screenshot(&self, path: Option<&str>) -> Result<()> {
        self.screenshot(path)
            .map(|_| ())
            .map_err(BrowserRenderError::from)
    }

    fn close(&self) -> Result<()> {
        self.close().map(|_| ()).map_err(BrowserRenderError::from)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserRenderError {
    #[error("workspace entry is not a file: {0}")]
    NotAFile(PathBuf),

    #[error("failed to convert workspace entry to file URL: {path}")]
    FileUrl {
        path: PathBuf,
        reason: Option<String>,
    },

    #[error(transparent)]
    AgentBrowser(#[from] ::agent_browser::BrowserError),
}

use async_trait::async_trait;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::error::Result;
use crate::protocol::{InputEvent, InputSource};

use super::IngressSource;

pub struct CliStdinIngress<R> {
    reader: R,
}

impl<R> CliStdinIngress<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

#[async_trait]
impl<R> IngressSource for CliStdinIngress<R>
where
    R: AsyncBufRead + Send + Sync + Unpin,
{
    fn name(&self) -> &str {
        "cli-stdin"
    }

    async fn next_event(&mut self) -> Result<Option<InputEvent>> {
        loop {
            let mut line = String::new();
            let bytes = self.reader.read_line(&mut line).await?;
            if bytes == 0 {
                tracing::debug!(ingress = self.name(), "stdin ingress reached eof");
                return Ok(None);
            }

            let text = line.trim().to_string();
            if text.is_empty() {
                tracing::debug!(ingress = self.name(), "stdin ingress skipped blank line");
                continue;
            }

            tracing::debug!(
                ingress = self.name(),
                bytes,
                text_len = text.len(),
                "stdin ingress produced user text event"
            );
            return Ok(Some(InputEvent::UserText {
                text,
                source: InputSource::Cli,
            }));
        }
    }
}

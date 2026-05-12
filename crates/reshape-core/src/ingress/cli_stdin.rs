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
                return Ok(None);
            }

            let text = line.trim().to_string();
            if text.is_empty() {
                continue;
            }

            return Ok(Some(InputEvent::UserText {
                text,
                source: InputSource::Cli,
            }));
        }
    }
}

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::Result;
use crate::protocol::{Envelope, InputEvent, OutputEvent};

#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish_inbound(&self, event: Envelope<InputEvent>) -> Result<()>;
    async fn publish_outbound(&self, event: Envelope<OutputEvent>) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct InProcessBus {
    inbound_tx: mpsc::Sender<Envelope<InputEvent>>,
    outbound_tx: mpsc::Sender<Envelope<OutputEvent>>,
}

impl InProcessBus {
    pub fn new(
        capacity: usize,
    ) -> (
        Self,
        mpsc::Receiver<Envelope<InputEvent>>,
        mpsc::Receiver<Envelope<OutputEvent>>,
    ) {
        let (inbound_tx, inbound_rx) = mpsc::channel(capacity);
        let (outbound_tx, outbound_rx) = mpsc::channel(capacity);

        (
            Self {
                inbound_tx,
                outbound_tx,
            },
            inbound_rx,
            outbound_rx,
        )
    }
}

#[async_trait]
impl EventBus for InProcessBus {
    async fn publish_inbound(&self, event: Envelope<InputEvent>) -> Result<()> {
        tracing::debug!(
            message_id = %event.header.message_id,
            trace_id = %event.header.trace_id,
            "publishing inbound event"
        );
        self.inbound_tx.send(event).await.map_err(|err| {
            tracing::error!(error = %err, "failed to publish inbound event");
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, err.to_string()).into()
        })
    }

    async fn publish_outbound(&self, event: Envelope<OutputEvent>) -> Result<()> {
        tracing::debug!(
            message_id = %event.header.message_id,
            trace_id = %event.header.trace_id,
            "publishing outbound event"
        );
        self.outbound_tx.send(event).await.map_err(|err| {
            tracing::error!(error = %err, "failed to publish outbound event");
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, err.to_string()).into()
        })
    }
}

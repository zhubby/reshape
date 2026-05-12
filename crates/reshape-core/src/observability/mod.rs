use async_trait::async_trait;

#[async_trait]
pub trait TelemetrySink: Send + Sync {
    async fn record_event(&self, name: &str);
}

#[derive(Debug, Default, Clone)]
pub struct NoopTelemetry;

#[async_trait]
impl TelemetrySink for NoopTelemetry {
    async fn record_event(&self, _name: &str) {}
}

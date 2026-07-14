use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::llm_client::LlmClient;
use crate::llm_client::StreamEventBox;
use crate::models::{MessageRequest, MessageResponse};

/// Object-safe model boundary for Engine dependency injection.
///
/// The existing `LlmClient` uses return-position `impl Future`, which is
/// efficient for concrete providers but cannot be placed behind `dyn`. This
/// adapter preserves that provider trait while giving deterministic Engine
/// tests and alternate adapters one injectable boundary.
#[async_trait]
#[allow(dead_code)]
pub trait ModelClient: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model(&self) -> &str;
    async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse>;
    async fn create_message_stream(&self, request: MessageRequest) -> Result<StreamEventBox>;
    async fn health_check(&self) -> Result<bool>;
}

pub type SharedModelClient = Arc<dyn ModelClient>;

/// Every existing provider client automatically satisfies the injectable
/// boundary. This keeps provider-specific HTTP/routing code behind
/// `LlmClient` while the Engine owns only the object-safe contract.
#[async_trait]
impl<T> ModelClient for T
where
    T: LlmClient + Send + Sync,
{
    fn provider_name(&self) -> &str {
        LlmClient::provider_name(self)
    }

    fn model(&self) -> &str {
        LlmClient::model(self)
    }

    async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse> {
        LlmClient::create_message(self, request).await
    }

    async fn create_message_stream(&self, request: MessageRequest) -> Result<StreamEventBox> {
        LlmClient::create_message_stream(self, request).await
    }

    async fn health_check(&self) -> Result<bool> {
        LlmClient::health_check(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_client_is_object_safe() {
        fn accepts_dyn(_: Option<SharedModelClient>) {}
        accepts_dyn(None);
    }
}

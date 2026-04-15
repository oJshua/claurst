use std::pin::Pin;
use std::sync::Arc;

use claurst_core::ProviderId;
use futures::Stream;

use crate::client::{AnthropicClient, ClientConfig};
use crate::provider::{
    LlmProvider, ModelId, ModelInfo, ProviderCapabilities, ProviderError, ProviderStatus,
    StreamEvent, SystemPromptStyle,
};
use crate::provider_types::{ProviderRequest, ProviderResponse, RequestActivity};
use crate::providers::AnthropicProvider;

/// Yolomax proxy provider.
///
/// Thin wrapper around `AnthropicProvider` that uses bearer auth, a custom
/// `api_base`, and injects `x-claurst-*` headers on every request.
/// The `x-claurst-activity` header is set per-request from
/// `ProviderRequest.activity`.
pub struct YolomaxProvider {
    inner: AnthropicProvider,
    id: ProviderId,
    session_id: String,
    client_version: String,
}

impl YolomaxProvider {
    pub fn new(
        api_base: String,
        bearer_token: String,
        session_id: String,
        client_version: String,
    ) -> Self {
        let mut extra_headers = std::collections::HashMap::new();
        extra_headers.insert(
            "x-claurst-client-version".to_string(),
            client_version.clone(),
        );
        extra_headers.insert("x-claurst-session-id".to_string(), session_id.clone());

        let config = ClientConfig {
            api_key: bearer_token,
            api_base,
            use_bearer_auth: true,
            extra_headers,
            ..Default::default()
        };

        Self {
            inner: AnthropicProvider::from_config(config),
            id: ProviderId::new(ProviderId::YOLOMAX),
            session_id,
            client_version,
        }
    }

    fn activity_headers(activity: RequestActivity) -> std::collections::HashMap<String, String> {
        let mut h = std::collections::HashMap::new();
        h.insert(
            "x-claurst-activity".to_string(),
            activity.as_header_value().to_string(),
        );
        h
    }
}

impl LlmProvider for YolomaxProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn name(&self) -> &str {
        "Yolomax"
    }

    async fn create_message(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        // Activity header is set per-request; the inner provider handles the
        // rest. For now, delegate directly — Task 6 will wire in the activity
        // header via the _with_headers path.
        self.inner.create_message(request).await
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        self.inner.create_message_stream(request).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.inner.list_models().await
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        self.inner.health_check().await
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
}

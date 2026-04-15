use std::pin::Pin;
use std::sync::Arc;

use async_stream::stream;
use claurst_core::types::{ContentBlock, UsageInfo};
use claurst_core::ProviderId;
use futures::{Stream, StreamExt};
use parking_lot::RwLock;

use crate::client::ClientConfig;
use crate::provider::{
    LlmProvider, ModelInfo, ProviderCapabilities, ProviderError, ProviderStatus, StreamEvent,
};
use crate::provider_types::{ProviderRequest, ProviderResponse, RequestActivity, StopReason};
use crate::providers::AnthropicProvider;
use crate::streaming::NullStreamHandler;

/// Yolomax proxy provider.
///
/// Thin wrapper around `AnthropicProvider` that uses bearer auth, a custom
/// `api_base`, and injects `x-claurst-*` headers on every request.
/// `x-claurst-activity` is set per-request from `ProviderRequest.activity`.
///
/// On 401 responses, transparently attempts a token refresh using the stored
/// refresh_token and retries the request once.
pub struct YolomaxProvider {
    inner: RwLock<AnthropicProvider>,
    id: ProviderId,
    api_base: String,
    extra_headers: std::collections::HashMap<String, String>,
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
            client_version,
        );
        extra_headers.insert("x-claurst-session-id".to_string(), session_id);

        let config = ClientConfig {
            api_key: bearer_token,
            api_base: api_base.clone(),
            use_bearer_auth: true,
            extra_headers: extra_headers.clone(),
            ..Default::default()
        };

        Self {
            inner: RwLock::new(AnthropicProvider::from_config(config)),
            id: ProviderId::new(ProviderId::YOLOMAX),
            api_base,
            extra_headers,
        }
    }

    fn activity_headers(activity: RequestActivity) -> std::collections::HashMap<String, String> {
        let mut h = std::collections::HashMap::with_capacity(1);
        h.insert(
            "x-claurst-activity".to_string(),
            activity.as_header_value().to_string(),
        );
        h
    }

    fn rebuild_inner(&self, new_token: &str) {
        let config = ClientConfig {
            api_key: new_token.to_string(),
            api_base: self.api_base.clone(),
            use_bearer_auth: true,
            extra_headers: self.extra_headers.clone(),
            ..Default::default()
        };
        *self.inner.write() = AnthropicProvider::from_config(config);
    }

    async fn try_refresh(&self) -> Result<(), String> {
        let refresh_token = claurst_core::yolomax_auth::stored_refresh_token()
            .ok_or_else(|| "No stored refresh token".to_string())?;
        let token_resp =
            claurst_core::yolomax_auth::refresh_access_token(&self.api_base, &refresh_token)
                .await?;
        self.rebuild_inner(&token_resp.access_token);
        Ok(())
    }

    fn is_auth_error(e: &ProviderError) -> bool {
        match e {
            ProviderError::AuthFailed { .. } => true,
            ProviderError::Other { status: Some(401), .. } => true,
            ProviderError::Other { message, .. } if message.contains("invalid_token") => true,
            _ => false,
        }
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
        let mut stream = self.create_message_stream(request.clone()).await;

        if let Err(ref e) = stream {
            if Self::is_auth_error(e) {
                if self.try_refresh().await.is_ok() {
                    stream = self.create_message_stream(request).await;
                } else {
                    return Err(ProviderError::Other {
                        provider: self.id.clone(),
                        message: "Yolomax token expired. Run /connect yolomax to re-authenticate."
                            .to_string(),
                        status: Some(401),
                        body: None,
                    });
                }
            }
        }

        let mut stream = stream?;

        let mut id = String::from("unknown");
        let mut model = String::new();
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut text_parts: Vec<(usize, String)> = Vec::new();
        let mut tool_buffers: std::collections::HashMap<usize, (String, String, String)> =
            std::collections::HashMap::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut usage = UsageInfo::default();

        while let Some(result) = stream.next().await {
            match result {
                Err(e) => return Err(e),
                Ok(evt) => match evt {
                    StreamEvent::MessageStart {
                        id: msg_id,
                        model: msg_model,
                        usage: msg_usage,
                    } => {
                        id = msg_id;
                        model = msg_model;
                        usage = msg_usage;
                    }
                    StreamEvent::ContentBlockStart { index, content_block } => match content_block {
                        ContentBlock::Text { text } => {
                            text_parts.push((index, text));
                        }
                        ContentBlock::ToolUse { id: tid, name, .. } => {
                            tool_buffers.insert(index, (tid, name, String::new()));
                        }
                        other => content_blocks.push(other),
                    },
                    StreamEvent::TextDelta { index, text } => {
                        if let Some(entry) = text_parts.iter_mut().find(|(i, _)| *i == index) {
                            entry.1.push_str(&text);
                        }
                    }
                    StreamEvent::InputJsonDelta { index, partial_json } => {
                        if let Some((_, _, buf)) = tool_buffers.get_mut(&index) {
                            buf.push_str(&partial_json);
                        }
                    }
                    StreamEvent::ContentBlockStop { index } => {
                        if let Some((tid, name, json_buf)) = tool_buffers.remove(&index) {
                            let input = serde_json::from_str(&json_buf)
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                            content_blocks.push(ContentBlock::ToolUse { id: tid, name, input });
                        }
                    }
                    StreamEvent::MessageDelta {
                        stop_reason: sr,
                        usage: delta_usage,
                    } => {
                        if let Some(r) = sr {
                            stop_reason = r;
                        }
                        if let Some(u) = delta_usage {
                            usage.output_tokens += u.output_tokens;
                        }
                    }
                    StreamEvent::MessageStop => break,
                    StreamEvent::Error { error_type, message } => {
                        return Err(ProviderError::StreamError {
                            provider: self.id.clone(),
                            message: format!("[{}] {}", error_type, message),
                            partial_response: None,
                        });
                    }
                    _ => {}
                },
            }
        }

        text_parts.sort_by_key(|(i, _)| *i);
        let mut all_blocks: Vec<(usize, ContentBlock)> = text_parts
            .into_iter()
            .map(|(i, text)| (i, ContentBlock::Text { text }))
            .collect();
        for block in content_blocks {
            all_blocks.push((usize::MAX, block));
        }
        all_blocks.sort_by_key(|(i, _)| *i);
        let content = all_blocks.into_iter().map(|(_, b)| b).collect();

        Ok(ProviderResponse {
            id,
            model,
            content,
            stop_reason,
            usage,
        })
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        let activity = request.activity;
        let api_request = AnthropicProvider::build_request(&request);
        let headers = Self::activity_headers(activity);
        let handler = Arc::new(NullStreamHandler);
        let client = self.inner.read().client().clone();
        let provider_id = self.id.clone();

        let mut rx = client
            .create_message_stream_with_headers(api_request, handler, &headers)
            .await
            .map_err(|e| {
                let status = if e.to_string().contains("401") {
                    Some(401)
                } else {
                    None
                };
                ProviderError::Other {
                    provider: provider_id.clone(),
                    message: e.to_string(),
                    status,
                    body: None,
                }
            })?;

        let s = stream! {
            while let Some(anthropic_evt) = rx.recv().await {
                if let Some(unified_evt) = AnthropicProvider::map_stream_event(anthropic_evt) {
                    yield Ok(unified_evt);
                }
            }
        };

        Ok(Box::pin(s))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.inner.read().list_models().await
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        self.inner.read().health_check().await
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.read().capabilities()
    }
}

use serde::Deserialize;

use crate::auth_store::{AuthStore, StoredCredential};

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
}

fn default_expires_in() -> u64 {
    900
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
    #[serde(default)]
    error_description: String,
}

pub async fn request_device_code(
    base_url: &str,
) -> Result<DeviceCodeResponse, String> {
    let url = format!("{}/v1/auth/device", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to reach Yolomax auth endpoint: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Yolomax device-code request failed ({}): {}",
            status, text
        ));
    }

    resp.json::<DeviceCodeResponse>()
        .await
        .map_err(|e| format!("Invalid device-code response: {}", e))
}

pub async fn poll_for_token(
    base_url: &str,
    device_code: &str,
    interval: u64,
    timeout_secs: u64,
) -> Result<TokenResponse, String> {
    let url = format!("{}/v1/auth/device/token", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let start = std::time::Instant::now();

    loop {
        if start.elapsed().as_secs() > timeout_secs {
            return Err("Timed out waiting for authorization".into());
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "device_code": device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }))
            .send()
            .await
            .map_err(|e| format!("Token poll failed: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

        if status.is_success() {
            if let Ok(token_resp) = serde_json::from_value::<TokenResponse>(body.clone()) {
                return Ok(token_resp);
            }
        }

        if let Some(error) = body.get("error").and_then(|v| v.as_str()) {
            match error {
                "authorization_pending" => continue,
                "slow_down" => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
                "expired_token" => return Err("Device code expired. Please try again.".into()),
                "access_denied" => return Err("Authorization was denied.".into()),
                other => {
                    let desc = body
                        .get("error_description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    return Err(format!("Auth error: {} {}", other, desc));
                }
            }
        }
    }
}

pub fn persist_tokens(tokens: &TokenResponse) {
    let mut store = AuthStore::load();
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + tokens.expires_in;

    store.set(
        "yolomax",
        StoredCredential::OAuthToken {
            access: tokens.access_token.clone(),
            refresh: tokens.refresh_token.clone(),
            expires: expires_at,
        },
    );
}

pub fn stored_refresh_token() -> Option<String> {
    let store = AuthStore::load();
    match store.get("yolomax")? {
        StoredCredential::OAuthToken { refresh, .. } if !refresh.is_empty() => {
            Some(refresh.clone())
        }
        _ => None,
    }
}

pub async fn refresh_access_token(
    base_url: &str,
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    let url = format!("{}/v1/auth/refresh", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Token refresh failed ({}): {}", status, text));
    }

    let token_resp: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Invalid refresh response: {}", e))?;

    persist_tokens(&token_resp);
    Ok(token_resp)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Relay HTTP client — thin reqwest blocking wrapper over the relay REST API.

use crate::core::error::KrillnotesError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Thin blocking HTTP client for the Krillnotes relay REST API.
///
/// All relay responses are wrapped as `{ "data": T }` JSON envelopes.
pub struct RelayClient {
    pub(crate) http: reqwest::blocking::Client,
    pub base_url: String,
    pub session_token: Option<String>,
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterChallenge {
    pub encrypted_nonce: String,
    pub server_public_key: String,
}

/// Result of a successful account registration.
#[derive(Debug, Deserialize)]
pub struct RegisterResult {
    pub account_id: String,
    pub challenge: RegisterChallenge,
}

#[derive(Debug, Deserialize)]
pub struct SessionResponse {
    pub session_token: String,
}

#[derive(Debug, Deserialize)]
pub struct DeviceKeyInfo {
    pub device_public_key: String,
    pub verified: bool,
    pub added_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AccountInfo {
    pub account_id: String,
    pub email: String,
    pub identity_uuid: String,
    pub device_keys: Vec<DeviceKeyInfo>,
    pub role: String,
    pub storage_used: u64,
}

#[derive(Debug, Deserialize)]
pub struct MailboxInfo {
    pub workspace_id: String,
    pub registered_at: String,
    pub pending_bundles: u32,
    pub storage_used: u64,
}

#[derive(Debug, Deserialize)]
pub struct BundleMeta {
    pub bundle_id: String,
    pub workspace_id: String,
    pub sender_device_key: String,
    pub mode: String,
    pub size_bytes: u64,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct InviteInfo {
    pub invite_id: String,
    pub token: String,
    pub url: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
pub struct InvitePayload {
    /// base64-encoded invite payload.
    pub payload: String,
    pub expires_at: String,
}

/// Wrapper for relay JSON responses: `{ "data": T }`.
#[derive(Debug, Deserialize)]
struct RelayResponse<T> {
    data: T,
}

// ── Upload/request helper types ─────────────────────────────────────────────

#[derive(Serialize)]
struct RegisterRequest<'a> {
    email: &'a str,
    password: &'a str,
    identity_uuid: &'a str,
    device_public_key: &'a str,
}

#[derive(Serialize)]
struct RegisterVerifyRequest<'a> {
    device_public_key: &'a str,
    nonce: &'a str,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    email: &'a str,
    password: &'a str,
    device_public_key: &'a str,
}

#[derive(Serialize)]
struct ResetPasswordRequest<'a> {
    email: &'a str,
}

#[derive(Serialize)]
struct ResetPasswordConfirmRequest<'a> {
    token: &'a str,
    new_password: &'a str,
}

#[derive(Serialize)]
struct AddDeviceRequest<'a> {
    device_public_key: &'a str,
}

#[derive(Serialize)]
struct VerifyDeviceRequest<'a> {
    device_public_key: &'a str,
    nonce: &'a str,
}

#[derive(Serialize)]
struct EnsureMailboxRequest<'a> {
    workspace_id: &'a str,
}

/// Routing header for a bundle upload. Serialised to JSON and sent as the `header` field.
#[derive(Serialize)]
pub struct BundleHeader {
    pub workspace_id: String,
    pub sender_device_key: String,
    pub sender_device_id: String,
    pub recipient_device_keys: Vec<String>,
    pub recipient_device_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Serialize)]
struct UploadBundleRequest {
    header: String,  // JSON-encoded BundleHeader
    payload: String, // base64
}

#[derive(Deserialize)]
struct UploadBundleSkipped {
    #[serde(default)]
    unknown: Vec<String>,
    #[serde(default)]
    unverified: Vec<String>,
    #[serde(default)]
    quota_exceeded: Vec<String>,
}

#[derive(Deserialize)]
struct UploadBundleResponse {
    #[allow(dead_code)]
    routed_to: u32,
    bundle_ids: Vec<String>,
    #[serde(default)]
    skipped: Option<UploadBundleSkipped>,
}

#[derive(Deserialize)]
struct BundleDownloadResponse {
    payload: String, // base64
}

#[derive(Serialize)]
struct CreateInviteRequest<'a> {
    payload: &'a str,
    expires_at: &'a str,
}

// ── Implementation ──────────────────────────────────────────────────────────

impl RelayClient {
    /// Create a new `RelayClient` for the given base URL (no trailing slash).
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::blocking::Client::builder()
                .user_agent(concat!("KrillNotes/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("failed to build HTTP client"),
            base_url: base_url.to_string(),
            session_token: None,
        }
    }

    /// Builder method: attach a session token.
    pub fn with_session_token(mut self, token: &str) -> Self {
        self.session_token = Some(token.to_string());
        self
    }

    /// Set the session token in place.
    pub fn set_session_token(&mut self, token: &str) {
        self.session_token = Some(token.to_string());
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth_header(&self) -> Result<String, KrillnotesError> {
        match &self.session_token {
            Some(tok) => Ok(format!("Bearer {tok}")),
            None => Err(KrillnotesError::RelayAuthExpired(
                "No session token set".to_string(),
            )),
        }
    }

    fn map_error(resp: reqwest::blocking::Response) -> KrillnotesError {
        let status = resp.status().as_u16();
        let body = resp.text().unwrap_or_default();
        // Extract human-readable message from {"error":{"message":"..."}} envelope.
        let message = Self::extract_error_message(&body)
            .unwrap_or_else(|| if body.is_empty() { format!("HTTP {status}") } else { body.clone() });
        match status {
            401 => KrillnotesError::RelayAuthExpired(message),
            404 | 410 => KrillnotesError::RelayNotFound(message),
            409 => KrillnotesError::RelayUnavailable(format!("HTTP 409: {message}")),
            429 => KrillnotesError::RelayRateLimited(message),
            _ => KrillnotesError::RelayUnavailable(format!("HTTP {status}: {message}")),
        }
    }

    /// Try to extract the human-readable message from the relay server's error envelope:
    /// `{"error":{"code":"...","message":"..."}}`
    fn extract_error_message(body: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(body).ok()?;
        v.get("error")?.get("message")?.as_str().map(str::to_string)
    }

    fn handle_response<T: DeserializeOwned>(
        resp: reqwest::blocking::Response,
    ) -> Result<T, KrillnotesError> {
        let status = resp.status();
        if status.is_success() {
            log::debug!(target: "krillnotes::relay", "response {status}");
            let wrapper: RelayResponse<T> = resp
                .json()
                .map_err(|e| KrillnotesError::RelayUnavailable(format!("invalid response JSON: {e}")))?;
            Ok(wrapper.data)
        } else {
            log::debug!(target: "krillnotes::relay", "error response {status}");
            Err(Self::map_error(resp))
        }
    }

    fn handle_empty(resp: reqwest::blocking::Response) -> Result<(), KrillnotesError> {
        let status = resp.status();
        if status.is_success() {
            log::debug!(target: "krillnotes::relay", "response {status}");
            Ok(())
        } else {
            log::debug!(target: "krillnotes::relay", "error response {status}");
            Err(Self::map_error(resp))
        }
    }

    // ── Auth endpoints ───────────────────────────────────────────────────────

    /// Register a new account. Returns the new account ID and a PoP challenge.
    pub fn register(
        &self,
        email: &str,
        password: &str,
        identity_uuid: &str,
        device_public_key: &str,
    ) -> Result<RegisterResult, KrillnotesError> {
        log::info!(target: "krillnotes::relay", "registering account for {email}");
        let body = RegisterRequest {
            email,
            password,
            identity_uuid,
            device_public_key,
        };
        log::debug!(target: "krillnotes::relay", "POST {}/auth/register", self.base_url);
        let resp = self
            .http
            .post(self.url("/auth/register"))
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "register request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// Verify registration by proving knowledge of the decrypted nonce.
    pub fn register_verify(
        &self,
        device_public_key: &str,
        nonce: &str,
    ) -> Result<SessionResponse, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/auth/register/verify", self.base_url);
        let body = RegisterVerifyRequest {
            device_public_key,
            nonce,
        };
        let resp = self
            .http
            .post(self.url("/auth/register/verify"))
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "register_verify request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// Log in with email, password, and device key. Returns a session token.
    pub fn login(&self, email: &str, password: &str, device_public_key: &str) -> Result<SessionResponse, KrillnotesError> {
        log::info!(target: "krillnotes::relay", "logging in as {email}");
        log::debug!(target: "krillnotes::relay", "POST {}/auth/login", self.base_url);
        let body = LoginRequest { email, password, device_public_key };
        let resp = self
            .http
            .post(self.url("/auth/login"))
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "login request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// Log out the current session.
    pub fn logout(&self) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/auth/logout", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .post(self.url("/auth/logout"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "logout request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_empty(resp)
    }

    /// Request a password reset email.
    pub fn reset_password(&self, email: &str) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/auth/reset-password", self.base_url);
        let body = ResetPasswordRequest { email };
        let resp = self
            .http
            .post(self.url("/auth/reset-password"))
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "reset_password request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_empty(resp)
    }

    /// Confirm password reset with a token and new password.
    pub fn reset_password_confirm(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/auth/reset-password/confirm", self.base_url);
        let body = ResetPasswordConfirmRequest { token, new_password };
        let resp = self
            .http
            .post(self.url("/auth/reset-password/confirm"))
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "reset_password_confirm request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_empty(resp)
    }

    // ── Account & Devices ────────────────────────────────────────────────────

    /// Fetch account information for the authenticated user.
    pub fn get_account(&self) -> Result<AccountInfo, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "GET {}/account", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .get(self.url("/account"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "get_account request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// Add an additional device key to the account. Returns a challenge.
    pub fn add_device(&self, device_public_key: &str) -> Result<RegisterChallenge, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/account/devices", self.base_url);
        let auth = self.auth_header()?;
        let body = AddDeviceRequest { device_public_key };
        let resp = self
            .http
            .post(self.url("/account/devices"))
            .header("Authorization", auth)
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "add_device request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// Verify a newly added device by proving knowledge of the challenge nonce.
    pub fn verify_device(&self, device_public_key: &str, nonce: &str) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/account/devices/verify", self.base_url);
        let auth = self.auth_header()?;
        let body = VerifyDeviceRequest { device_public_key, nonce };
        let resp = self
            .http
            .post(self.url("/account/devices/verify"))
            .header("Authorization", auth)
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "verify_device request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_empty(resp)
    }

    // ── Mailboxes ────────────────────────────────────────────────────────────

    /// Ensure a mailbox exists for the given workspace (idempotent — 200/201 both ok).
    pub fn ensure_mailbox(&self, workspace_id: &str) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/mailboxes (workspace_id={workspace_id})", self.base_url);
        let auth = self.auth_header()?;
        let body = EnsureMailboxRequest { workspace_id };
        let resp = self
            .http
            .post(self.url("/mailboxes"))
            .header("Authorization", auth)
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "ensure_mailbox request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        // 200 or 201 both indicate success
        let status = resp.status().as_u16();
        if status == 200 || status == 201 {
            log::debug!(target: "krillnotes::relay", "mailbox ensured (HTTP {status})");
            Ok(())
        } else {
            log::error!(target: "krillnotes::relay", "ensure_mailbox failed (HTTP {status})");
            Err(Self::map_error(resp))
        }
    }

    /// List all mailboxes for the authenticated account.
    pub fn list_mailboxes(&self) -> Result<Vec<MailboxInfo>, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "GET {}/mailboxes", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .get(self.url("/mailboxes"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "list_mailboxes request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    // ── Bundles ──────────────────────────────────────────────────────────────

    /// Upload a bundle. Returns the list of bundle IDs created (one per routed recipient).
    pub fn upload_bundle(
        &self,
        header: &BundleHeader,
        bundle_bytes: &[u8],
    ) -> Result<Vec<String>, KrillnotesError> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        log::debug!(target: "krillnotes::relay", "POST {}/bundles ({} bytes, {} recipients)", self.base_url, bundle_bytes.len(), header.recipient_device_keys.len());
        let auth = self.auth_header()?;
        let payload = BASE64.encode(bundle_bytes);
        let header_json = serde_json::to_string(header).map_err(|e| {
            KrillnotesError::RelayUnavailable(format!("failed to serialize bundle header: {e}"))
        })?;
        let body = UploadBundleRequest { header: header_json, payload };
        let resp = self
            .http
            .post(self.url("/bundles"))
            .header("Authorization", auth)
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "upload_bundle request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        let result: UploadBundleResponse = Self::handle_response(resp)?;
        log::info!(target: "krillnotes::relay", "uploaded bundle, {} bundle IDs created", result.bundle_ids.len());
        if let Some(skipped) = &result.skipped {
            if !skipped.unknown.is_empty() {
                log::warn!(target: "krillnotes::relay", "relay skipped unknown device keys: {:?}", skipped.unknown);
            }
            if !skipped.unverified.is_empty() {
                log::warn!(target: "krillnotes::relay", "relay skipped unverified device keys: {:?}", skipped.unverified);
            }
            if !skipped.quota_exceeded.is_empty() {
                log::warn!(target: "krillnotes::relay", "relay skipped device keys (quota exceeded): {:?}", skipped.quota_exceeded);
            }
        }
        Ok(result.bundle_ids)
    }

    /// List all pending bundles for the authenticated account.
    ///
    /// `device_id` is sent as a `?device_id=` query parameter so the relay can
    /// filter bundles to the specific device (Task D relay-server changes will
    /// honour this; the current server ignores it but still returns 200).
    pub fn list_bundles(&self, device_id: &str) -> Result<Vec<BundleMeta>, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "GET {}/bundles (device_id={device_id})", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .get(self.url("/bundles"))
            .header("Authorization", auth)
            .query(&[("device_id", device_id)])
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "list_bundles request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        let bundles: Vec<BundleMeta> = Self::handle_response(resp)?;
        log::info!(target: "krillnotes::relay", "listed {} pending bundles", bundles.len());
        Ok(bundles)
    }

    /// Download a bundle by ID. Returns the raw bytes.
    pub fn download_bundle(&self, bundle_id: &str) -> Result<Vec<u8>, KrillnotesError> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        log::debug!(target: "krillnotes::relay", "GET {}/bundles/{bundle_id}", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .get(self.url(&format!("/bundles/{bundle_id}")))
            .header("Authorization", auth)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "download_bundle request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        let result: BundleDownloadResponse = Self::handle_response(resp)?;
        let data = BASE64.decode(&result.payload).map_err(|e| {
            log::error!(target: "krillnotes::relay", "invalid bundle payload base64: {e}");
            KrillnotesError::RelayUnavailable(format!("invalid bundle payload base64: {e}"))
        })?;
        log::debug!(target: "krillnotes::relay", "downloaded bundle {bundle_id} ({} bytes)", data.len());
        Ok(data)
    }

    /// Delete a bundle by ID.
    pub fn delete_bundle(&self, bundle_id: &str) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "DELETE {}/bundles/{bundle_id}", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .delete(self.url(&format!("/bundles/{bundle_id}")))
            .header("Authorization", auth)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "delete_bundle request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_empty(resp)
    }

    // ── Invites ──────────────────────────────────────────────────────────────

    /// Create an invite with a base64-encoded payload and expiry timestamp.
    pub fn create_invite(
        &self,
        payload_base64: &str,
        expires_at: &str,
    ) -> Result<InviteInfo, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "POST {}/invites", self.base_url);
        let auth = self.auth_header()?;
        let body = CreateInviteRequest {
            payload: payload_base64,
            expires_at,
        };
        let resp = self
            .http
            .post(self.url("/invites"))
            .header("Authorization", auth)
            .json(&body)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "create_invite request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// List all active invites for the authenticated account.
    pub fn list_invites(&self) -> Result<Vec<InviteInfo>, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "GET {}/invites", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .get(self.url("/invites"))
            .header("Authorization", auth)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "list_invites request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// Fetch an invite payload by token (no auth required).
    pub fn fetch_invite(&self, token: &str) -> Result<InvitePayload, KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "GET {}/invites/<token>", self.base_url);
        let resp = self
            .http
            .get(self.url(&format!("/invites/{token}")))
            .header("Accept", "application/json")
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "fetch_invite request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_response(resp)
    }

    /// Delete an invite by token.
    pub fn delete_invite(&self, token: &str) -> Result<(), KrillnotesError> {
        log::debug!(target: "krillnotes::relay", "DELETE {}/invites/<token>", self.base_url);
        let auth = self.auth_header()?;
        let resp = self
            .http
            .delete(self.url(&format!("/invites/{token}")))
            .header("Authorization", auth)
            .send()
            .map_err(|e| {
                log::error!(target: "krillnotes::relay", "delete_invite request failed: {e}");
                KrillnotesError::RelayUnavailable(e.to_string())
            })?;
        Self::handle_empty(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_client_construction() {
        let client = RelayClient::new("https://relay.example.com");
        assert_eq!(client.base_url, "https://relay.example.com");
        assert!(client.session_token.is_none());
    }

    #[test]
    fn test_relay_client_with_token() {
        let client = RelayClient::new("https://relay.example.com")
            .with_session_token("tok_abc123");
        assert_eq!(client.session_token.as_deref(), Some("tok_abc123"));
    }

    #[test]
    fn test_relay_client_set_session_token() {
        let mut client = RelayClient::new("https://relay.example.com");
        client.set_session_token("tok_xyz");
        assert_eq!(client.session_token.as_deref(), Some("tok_xyz"));
    }

    #[test]
    fn test_relay_client_auth_header_no_token() {
        let client = RelayClient::new("https://relay.example.com");
        assert!(client.auth_header().is_err());
    }

    #[test]
    fn test_relay_client_auth_header_with_token() {
        let client = RelayClient::new("https://relay.example.com")
            .with_session_token("tok_abc123");
        assert_eq!(client.auth_header().unwrap(), "Bearer tok_abc123");
    }

    #[test]
    fn test_relay_client_url_construction() {
        let client = RelayClient::new("https://relay.example.com");
        assert_eq!(client.url("/auth/login"), "https://relay.example.com/auth/login");
    }
}

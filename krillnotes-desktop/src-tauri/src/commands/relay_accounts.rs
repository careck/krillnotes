// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Tauri commands for relay account CRUD.
//!
//! These commands expose the per-identity [`RelayAccountManager`] to the
//! frontend, allowing users to register with relay servers, log in, list
//! their relay accounts, and delete them.

use crate::AppState;
use chrono::Utc;
use serde::Serialize;
use tauri::{State, Window};
use uuid::Uuid;

use krillnotes_core::core::sync::relay::RelayClient;
use krillnotes_core::core::sync::relay::auth::decrypt_pop_challenge;

/// Relay account info returned to the frontend.
///
/// Never exposes the password or session token — only derived session validity.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayAccountInfo {
    pub relay_account_id: String,
    pub relay_url: String,
    pub email: String,
    /// `true` when the stored session token has not yet expired.
    pub session_valid: bool,
}

impl RelayAccountInfo {
    fn from_account(a: &krillnotes_core::core::sync::relay::RelayAccount) -> Self {
        Self {
            relay_account_id: a.relay_account_id.to_string(),
            relay_url: a.relay_url.clone(),
            email: a.email.clone(),
            session_valid: a.session_expires_at > Utc::now(),
        }
    }
}

// ── list_relay_accounts ────────────────────────────────────────────────────

/// List all relay accounts for the given identity.
#[tauri::command]
pub fn list_relay_accounts(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> Result<Vec<RelayAccountInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let managers = state.relay_account_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let accounts = mgr.list_relay_accounts().map_err(|e| {
        log::error!("list_relay_accounts failed: {e}");
        e.to_string()
    })?;
    Ok(accounts.iter().map(RelayAccountInfo::from_account).collect())
}

// ── register_relay_account ─────────────────────────────────────────────────

/// Register a new account on a relay server and store the credentials.
///
/// Performs the full register → PoP challenge → verify flow, then persists
/// the resulting session via the `RelayAccountManager`.
#[tauri::command]
pub async fn register_relay_account(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<RelayAccountInfo, String> {
    log::debug!("register_relay_account(identity={identity_uuid}, relay_url={relay_url})");
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Capture signing key and device public key while holding brief lock.
    let (signing_key, device_public_key) = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        let id = m.get(&uuid)
            .ok_or("Identity is not unlocked — please unlock your identity first")?;
        let sk = id.signing_key.clone();
        let dpk = hex::encode(id.verifying_key.to_bytes());
        (sk, dpk)
    };

    let identity_uuid_str = identity_uuid.clone();
    let relay_url_clone = relay_url.clone();
    let email_clone = email.clone();
    let password_clone = password.clone();
    let dpk = device_public_key.clone();

    // RelayClient uses reqwest::blocking — must run in spawn_blocking.
    let (session_token, session_expires_at) = tokio::task::spawn_blocking(move || {
        let client = RelayClient::new(&relay_url_clone);

        // Step 1: Register → receive PoP challenge.
        let result = client
            .register(&email_clone, &password_clone, &identity_uuid_str, &dpk)
            .map_err(|e| e.to_string())?;

        // Step 2: Decrypt the PoP challenge.
        let nonce_bytes = decrypt_pop_challenge(
            &signing_key,
            &result.challenge.encrypted_nonce,
            &result.challenge.server_public_key,
        )
        .map_err(|e| e.to_string())?;
        let nonce_hex = hex::encode(&nonce_bytes);

        // Step 3: Verify registration → obtain session token.
        let session = client
            .register_verify(&dpk, &nonce_hex)
            .map_err(|e| e.to_string())?;

        let expires = Utc::now() + chrono::Duration::days(30);
        Ok::<_, String>((session.session_token, expires))
    })
    .await
    .map_err(|e| {
        log::error!("register_relay_account spawn_blocking join failed: {e}");
        e.to_string()
    })??;

    // Store via RelayAccountManager.
    let managers = state.relay_account_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let account = mgr
        .create_relay_account(
            &relay_url,
            &email,
            &password,
            &session_token,
            session_expires_at,
            &device_public_key,
        )
        .map_err(|e| {
            log::error!("register_relay_account: create_relay_account failed: {e}");
            e.to_string()
        })?;

    Ok(RelayAccountInfo::from_account(&account))
}

// ── login_relay_account ────────────────────────────────────────────────────

/// Re-authenticate with an existing relay account (e.g. after token expiry).
///
/// If an account for this URL already exists it is updated in place;
/// otherwise a new account entry is created.
#[tauri::command]
pub async fn login_relay_account(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<RelayAccountInfo, String> {
    log::debug!("login_relay_account(identity={identity_uuid}, relay_url={relay_url})");
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let device_public_key = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        let id = m.get(&uuid)
            .ok_or("Identity is not unlocked — please unlock your identity first")?;
        hex::encode(id.verifying_key.to_bytes())
    };

    let relay_url_clone = relay_url.clone();
    let email_clone = email.clone();
    let password_clone = password.clone();
    let dpk = device_public_key.clone();

    // RelayClient uses reqwest::blocking — must run in spawn_blocking.
    let session_token = tokio::task::spawn_blocking(move || {
        let client = RelayClient::new(&relay_url_clone);
        let session = client
            .login(&email_clone, &password_clone, &dpk)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(session.session_token)
    })
    .await
    .map_err(|e| {
        log::error!("login_relay_account spawn_blocking join failed: {e}");
        e.to_string()
    })??;

    let session_expires_at = Utc::now() + chrono::Duration::days(30);

    // Update existing account or create a new one.
    let managers = state.relay_account_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;

    let account = if let Some(mut existing) = mgr.find_by_url(&relay_url).map_err(|e| e.to_string())? {
        // Update the existing account with the new session.
        existing.email = email;
        existing.password = password;
        existing.session_token = session_token;
        existing.session_expires_at = session_expires_at;
        existing.device_public_key = device_public_key;
        mgr.save_relay_account(&existing).map_err(|e| {
            log::error!("login_relay_account: save_relay_account failed: {e}");
            e.to_string()
        })?;
        existing
    } else {
        mgr.create_relay_account(
            &relay_url,
            &email,
            &password,
            &session_token,
            session_expires_at,
            &device_public_key,
        )
        .map_err(|e| {
            log::error!("login_relay_account: create_relay_account failed: {e}");
            e.to_string()
        })?
    };

    Ok(RelayAccountInfo::from_account(&account))
}

// ── delete_relay_account ───────────────────────────────────────────────────

/// Delete a relay account by ID.
#[tauri::command]
pub fn delete_relay_account(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_account_id: String,
) -> Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let account_id = Uuid::parse_str(&relay_account_id).map_err(|e| e.to_string())?;
    let managers = state.relay_account_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    mgr.delete_relay_account(account_id).map_err(|e| {
        log::error!("delete_relay_account failed: {e}");
        e.to_string()
    })
}

// ── set_peer_relay ─────────────────────────────────────────────────────────

/// Assign a relay account as the sync channel for a workspace peer.
///
/// Writes `channel_type = "relay"` and `channel_params = {"relay_account_id":"<uuid>"}`
/// into the peer's entry in the workspace database.
#[tauri::command]
pub fn set_peer_relay(
    window: Window,
    state: State<'_, AppState>,
    peer_device_id: String,
    relay_account_id: String,
) -> Result<(), String> {
    log::debug!("set_peer_relay(peer={peer_device_id}, relay_account_id={relay_account_id})");
    // Validate UUID to prevent malformed JSON
    let _acct_uuid = Uuid::parse_str(&relay_account_id).map_err(|e| e.to_string())?;
    let workspace_label = window.label().to_string();
    let channel_params = serde_json::json!({ "relay_account_id": relay_account_id }).to_string();
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces
        .get(&workspace_label)
        .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?;
    ws.update_peer_channel(&peer_device_id, "relay", &channel_params)
        .map_err(|e| {
            log::error!("set_peer_relay(peer={peer_device_id}) failed: {e}");
            e.to_string()
        })
}

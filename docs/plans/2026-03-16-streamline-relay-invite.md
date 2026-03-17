# Streamline Relay Invite Workflow — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One-click "Share Invite Link" that creates an invite, uploads to relay, and copies the URL — with full relay round-trip support for both inviter and invitee.

**Architecture:** Add `relay_url` to `InviteRecord`, add in-memory ZIP serialization methods, implement 4 new Tauri commands + 2 stub replacements, and add relay link buttons to WorkspacePeersDialog, InviteManagerDialog, and ImportInviteDialog.

**Tech Stack:** Rust (krillnotes-core), Tauri v2, React 19, TypeScript, reqwest (blocking), zip crate

**Spec:** `docs/plans/2026-03-16-streamline-relay-invite-design.md`

---

## Chunk 1: Core Foundation

### Task 1: Add `relay_url` to `InviteRecord`

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs:22-32`
- Test: `krillnotes-core/src/core/invite.rs` (existing test module)

- [ ] **Step 1: Write test for backward-compatible deserialization**

In the test module at the bottom of `invite.rs`, add:

```rust
#[test]
fn invite_record_deserializes_without_relay_url() {
    let json = r#"{
        "inviteId": "00000000-0000-0000-0000-000000000001",
        "workspaceId": "ws-1",
        "workspaceName": "Test",
        "createdAt": "2026-01-01T00:00:00Z",
        "expiresAt": null,
        "revoked": false,
        "useCount": 0
    }"#;
    let record: InviteRecord = serde_json::from_str(json).unwrap();
    assert!(record.relay_url.is_none());
}

#[test]
fn invite_record_deserializes_with_relay_url() {
    let json = r#"{
        "inviteId": "00000000-0000-0000-0000-000000000001",
        "workspaceId": "ws-1",
        "workspaceName": "Test",
        "createdAt": "2026-01-01T00:00:00Z",
        "expiresAt": null,
        "revoked": false,
        "useCount": 0,
        "relayUrl": "https://swarm.krillnotes.org/invites/abc123"
    }"#;
    let record: InviteRecord = serde_json::from_str(json).unwrap();
    assert_eq!(record.relay_url.as_deref(), Some("https://swarm.krillnotes.org/invites/abc123"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core invite_record_deserializes`
Expected: FAIL — `InviteRecord` has no `relay_url` field

- [ ] **Step 3: Add `relay_url` field to `InviteRecord`**

In `invite.rs`, add to the `InviteRecord` struct (after `use_count`):

```rust
    #[serde(default)]
    pub relay_url: Option<String>,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core invite_record_deserializes`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "feat(invite): add relay_url field to InviteRecord"
```

---

### Task 2: Add `set_relay_url` method to `InviteManager`

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn set_relay_url_persists() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = InviteManager::new(dir.path().to_path_buf()).unwrap();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);

    // Create an invite via the public API
    let (record, _invite_file) = mgr.create_invite(
        "ws-1", "Test", Some(7), &signing_key, "Alice",
        None, None, None, None, None, vec![],
    ).unwrap();
    let id = record.invite_id;
    assert!(record.relay_url.is_none());

    // Set relay URL
    mgr.set_relay_url(id, "https://swarm.krillnotes.org/invites/abc".into()).unwrap();

    // Read back
    let loaded = mgr.get_invite(id).unwrap().unwrap();
    assert_eq!(loaded.relay_url.as_deref(), Some("https://swarm.krillnotes.org/invites/abc"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core set_relay_url_persists`
Expected: FAIL — method not found

- [ ] **Step 3: Implement `set_relay_url`**

Add to `InviteManager` impl block:

```rust
    /// Set the relay URL on an existing invite record.
    pub fn set_relay_url(&mut self, invite_id: Uuid, url: String) -> Result<()> {
        let path = self.path_for(invite_id);
        let json = std::fs::read_to_string(&path)
            .map_err(|_| KrillnotesError::NotFound(format!("Invite {invite_id} not found")))?;
        let mut record: InviteRecord = serde_json::from_str(&json)?;
        record.relay_url = Some(url);
        self.save_record(&record)?;
        Ok(())
    }
```

Note: `save_record` is currently private. If it's `fn save_record` (no `pub`), this is fine since it's in the same impl block.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core set_relay_url_persists`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "feat(invite): add set_relay_url method to InviteManager"
```

---

### Task 3: Refactor ZIP helpers for in-memory support

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs:142-170` (write_json_zip, read_json_from_zip)

The existing `write_json_zip` writes to `std::fs::File`. Refactor to accept any `Write + Seek` so we can use `Cursor<Vec<u8>>` for in-memory serialization.

- [ ] **Step 1: Write test for in-memory ZIP round-trip**

```rust
#[test]
fn zip_round_trip_in_memory() {
    use std::io::Cursor;
    let data = r#"{"hello":"world"}"#;
    let entry_name = "test.json";

    let mut buf = Cursor::new(Vec::new());
    write_json_zip_to_writer(&mut buf, entry_name, data).unwrap();
    let bytes = buf.into_inner();

    let content = read_json_from_zip_bytes(&bytes, entry_name).unwrap();
    assert_eq!(content, data);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core zip_round_trip_in_memory`
Expected: FAIL — functions don't exist

- [ ] **Step 3: Refactor ZIP helpers**

Replace the existing helpers with generic versions:

```rust
/// Write a single JSON entry into a ZIP archive on any writer.
fn write_json_zip_to_writer<W: Write + Seek>(
    writer: W,
    entry_name: &str,
    json: &str,
) -> Result<()> {
    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(entry_name, options)
        .map_err(|e| KrillnotesError::Swarm(format!("zip write error: {e}")))?;
    zip.write_all(json.as_bytes())?;
    zip.finish()
        .map_err(|e| KrillnotesError::Swarm(format!("zip finish error: {e}")))?;
    Ok(())
}

/// Write a single JSON entry into a ZIP file on disk.
fn write_json_zip(path: &Path, entry_name: &str, json: &str) -> Result<()> {
    let file = std::fs::File::create(path)?;
    write_json_zip_to_writer(file, entry_name, json)
}

/// Read a named entry from ZIP bytes in memory.
fn read_json_from_zip_bytes(bytes: &[u8], entry_name: &str) -> Result<String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| KrillnotesError::Swarm(format!("Cannot read .swarm bytes: {e}")))?;
    let mut file = archive.by_name(entry_name)
        .map_err(|e| KrillnotesError::Swarm(format!("Missing {entry_name} in archive: {e}")))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}
```

Keep the existing `read_json_from_zip(path, entry_name)` but have it read bytes and delegate to `read_json_from_zip_bytes`.

- [ ] **Step 4: Run all invite tests to verify nothing broke**

Run: `cargo test -p krillnotes-core` (full crate test)
Expected: All existing tests PASS + new test PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "refactor(invite): extract generic ZIP helpers for in-memory support"
```

---

### Task 4: Add `serialize_invite_to_bytes` and `serialize_response_to_bytes`

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn serialize_and_parse_invite_bytes_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = InviteManager::new(dir.path().to_path_buf()).unwrap();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);

    let (_record, invite_file) = mgr.create_invite(
        "ws-1", "Test Workspace", Some(7),
        &signing_key, "Alice",
        None, None, None, None, None, vec![],
    ).unwrap();

    let bytes = InviteManager::serialize_invite_to_bytes(&invite_file).unwrap();
    assert!(!bytes.is_empty());

    // Should be valid ZIP
    let parsed = InviteManager::parse_and_verify_invite_bytes(&bytes).unwrap();
    assert_eq!(parsed.invite_id, invite_file.invite_id);
    assert_eq!(parsed.workspace_name, "Test Workspace");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core serialize_and_parse_invite_bytes`
Expected: FAIL — methods don't exist

- [ ] **Step 3: Implement `serialize_invite_to_bytes`**

Add to `InviteManager` impl:

```rust
    /// Serialize an InviteFile to ZIP bytes in memory (no file I/O).
    pub fn serialize_invite_to_bytes(file: &InviteFile) -> Result<Vec<u8>> {
        let json = serde_json::to_string_pretty(file)?;
        let mut cursor = std::io::Cursor::new(Vec::new());
        write_json_zip_to_writer(&mut cursor, "invite.json", &json)?;
        Ok(cursor.into_inner())
    }
```

- [ ] **Step 4: Implement `parse_and_verify_invite_bytes`**

Extract shared logic from `parse_and_verify_invite`. The existing `verify_payload` takes `(&Value, &str, &str)` where the third arg is the base64 public key string — follow the existing pattern exactly:

```rust
    /// Parse and verify an invite from raw ZIP bytes.
    pub fn parse_and_verify_invite_bytes(bytes: &[u8]) -> Result<InviteFile> {
        let json = read_json_from_zip_bytes(bytes, "invite.json")?;
        Self::verify_and_parse_invite_json(&json)
    }

    /// Parse and verify an invite from a file path.
    pub fn parse_and_verify_invite(path: &Path) -> Result<InviteFile> {
        let json = read_json_from_zip(path, "invite.json")?;
        Self::verify_and_parse_invite_json(&json)
    }

    /// Shared: parse JSON + verify signature.
    fn verify_and_parse_invite_json(json: &str) -> Result<InviteFile> {
        let invite: InviteFile = serde_json::from_str(json)?;
        let payload = serde_json::to_value(&invite)?;
        verify_payload(&payload, &invite.signature, &invite.inviter_public_key)?;
        Ok(invite)
    }
```

- [ ] **Step 5: Add `serialize_response_to_bytes`**

```rust
    /// Serialize an InviteResponseFile to ZIP bytes in memory.
    pub fn serialize_response_to_bytes(response: &InviteResponseFile) -> Result<Vec<u8>> {
        let json = serde_json::to_string_pretty(response)?;
        let mut cursor = std::io::Cursor::new(Vec::new());
        write_json_zip_to_writer(&mut cursor, "response.json", &json)?;
        Ok(cursor.into_inner())
    }
```

And add `parse_and_verify_response_bytes` using the same pattern:

```rust
    /// Parse and verify a response from raw ZIP bytes.
    pub fn parse_and_verify_response_bytes(bytes: &[u8]) -> Result<InviteResponseFile> {
        let json = read_json_from_zip_bytes(bytes, "response.json")?;
        Self::verify_and_parse_response_json(&json)
    }

    /// Shared: parse response JSON + verify signature.
    fn verify_and_parse_response_json(json: &str) -> Result<InviteResponseFile> {
        let response: InviteResponseFile = serde_json::from_str(json)?;
        let payload = serde_json::to_value(&response)?;
        verify_payload(&payload, &response.signature, &response.invitee_public_key)?;
        Ok(response)
    }
```

Refactor existing `parse_and_verify_response(path)` to call `verify_and_parse_response_json` internally.

- [ ] **Step 6: Run tests**

Run: `cargo test -p krillnotes-core`
Expected: All PASS

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "feat(invite): add in-memory serialize/parse for invites and responses"
```

---

### Task 5: Refactor `build_and_save_response` into `build_response` + save

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs:338-358`

- [ ] **Step 1: Write test for `build_response`**

```rust
#[test]
fn build_response_returns_signed_response() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = InviteManager::new(dir.path().to_path_buf()).unwrap();
    let inviter_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let invitee_key = SigningKey::generate(&mut rand::rngs::OsRng);

    let (_record, invite_file) = mgr.create_invite(
        "ws-1", "Test", Some(7), &inviter_key, "Alice",
        None, None, None, None, None, vec![],
    ).unwrap();

    let response = InviteManager::build_response(&invite_file, &invitee_key, "Bob").unwrap();
    assert_eq!(response.invite_id, invite_file.invite_id);
    assert_eq!(response.invitee_declared_name, "Bob");
    assert_eq!(response.file_type, "krillnotes-invite-response-v1");
    assert!(!response.signature.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core build_response_returns`
Expected: FAIL

- [ ] **Step 3: Extract `build_response` from `build_and_save_response`**

```rust
    /// Build a signed InviteResponseFile without saving to disk.
    pub fn build_response(
        invite: &InviteFile,
        signing_key: &SigningKey,
        declared_name: &str,
    ) -> Result<InviteResponseFile> {
        let invitee_public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().as_bytes());

        let mut response = InviteResponseFile {
            file_type: "krillnotes-invite-response-v1".into(),
            invite_id: invite.invite_id.clone(),
            invitee_public_key,
            invitee_declared_name: declared_name.into(),
            signature: String::new(),
        };
        let payload = serde_json::to_value(&response)?;
        response.signature = sign_payload(&payload, signing_key);
        Ok(response)
    }

    /// Build a signed response and save it to a .swarm file.
    pub fn build_and_save_response(
        invite: &InviteFile,
        signing_key: &SigningKey,
        declared_name: &str,
        save_path: &Path,
    ) -> Result<()> {
        let response = Self::build_response(invite, signing_key, declared_name)?;
        let json = serde_json::to_string_pretty(&response)?;
        write_json_zip(save_path, "response.json", &json)?;
        Ok(())
    }
```

- [ ] **Step 4: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "refactor(invite): extract build_response from build_and_save_response"
```

---

## Chunk 2: Tauri Commands

### Task 6: Update `InviteInfo` DTO with `relay_url`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/invites.rs:12-35`

- [ ] **Step 1: Add `relay_url` field to `InviteInfo` struct**

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfo {
    pub invite_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked: bool,
    pub use_count: u32,
    pub relay_url: Option<String>,
}
```

- [ ] **Step 2: Update `From<InviteRecord>` impl**

```rust
impl From<krillnotes_core::core::invite::InviteRecord> for InviteInfo {
    fn from(r: krillnotes_core::core::invite::InviteRecord) -> Self {
        Self {
            invite_id: r.invite_id.to_string(),
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            created_at: r.created_at.to_rfc3339(),
            expires_at: r.expires_at.map(|d| d.to_rfc3339()),
            revoked: r.revoked,
            use_count: r.use_count,
            relay_url: r.relay_url,
        }
    }
}
```

- [ ] **Step 3: Add `FetchedRelayInvite` struct**

Add near `InviteInfo`:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedRelayInvite {
    pub invite: InviteFileData,
    pub temp_path: String,
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p krillnotes-desktop`
Expected: PASS (no errors)

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/invites.rs
git commit -m "feat(invite): add relay_url to InviteInfo DTO, add FetchedRelayInvite"
```

---

### Task 7: Implement `share_invite_link` command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs`

This is the one-click command: create invite + upload to relay + return URL.

- [ ] **Step 1: Add the command**

Add after the existing `has_relay_credentials` function. The implementation follows the same pattern as `create_invite` in `commands/invites.rs` for getting signing key + workspace metadata, then adds relay upload:

```rust
use crate::commands::invites::InviteInfo;
use krillnotes_core::core::invite::InviteManager;
use krillnotes_core::core::sync::relay::client::RelayClient;

#[tauri::command]
pub async fn share_invite_link(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_name: String,
    expires_in_days: Option<u32>,
) -> Result<InviteInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // 1. Get signing key + declared name
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // 2. Get workspace metadata
    let (ws_id, ws_desc, ws_author, ws_org, ws_url, ws_license, ws_tags) = {
        let wss = state.workspaces.lock().expect("Mutex poisoned");
        let ws = wss.get(window.label()).ok_or("No workspace open")?;
        let meta = ws.get_workspace_metadata().map_err(|e| e.to_string())?;
        (
            ws.workspace_id().to_string(),
            meta.description,
            meta.author_name,
            meta.author_org,
            meta.homepage_url,
            meta.license,
            meta.tags,
        )
    };

    // 3. Create invite record + InviteFile
    let (record, invite_file) = {
        let mut mgrs = state.invite_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get_mut(&uuid).ok_or("No invite manager for identity")?;
        mgr.create_invite(
            &ws_id, &workspace_name, expires_in_days,
            &signing_key, &declared_name,
            ws_desc, ws_author, ws_org, ws_url, ws_license, ws_tags,
        ).map_err(|e| e.to_string())?
    };

    // 4. Serialize to bytes in memory
    let bytes = InviteManager::serialize_invite_to_bytes(&invite_file)
        .map_err(|e| e.to_string())?;
    let payload_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    // 5. Compute expires_at ISO 8601
    let expires_at_str = record.expires_at
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| {
            // Default to 90 days (relay max) if no expiry set
            (chrono::Utc::now() + chrono::Duration::days(90)).to_rfc3339()
        });

    // 6. Get relay account + build client
    let (relay_url, email, password, session_token, device_pub_key) = {
        let mgrs = state.relay_account_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get(&uuid).ok_or("No relay accounts for identity")?;
        let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
        let acct = accounts.first().ok_or("No relay account configured")?;
        (
            acct.relay_url.clone(),
            acct.email.clone(),
            acct.password.clone(),
            acct.session_token.clone(),
            acct.device_public_key.clone(),
        )
    };

    // 7. Upload to relay (on blocking thread)
    let invite_id = record.invite_id;
    let relay_result = tokio::task::spawn_blocking(move || {
        let mut client = RelayClient::new(&relay_url)
            .with_session_token(session_token);

        // Try upload; if 401, auto-login and retry
        match client.create_invite(&payload_base64, &expires_at_str) {
            Ok(info) => Ok(info.url),
            Err(krillnotes_core::KrillnotesError::RelayAuthExpired(_)) => {
                // Auto-login
                let session = client.login(&email, &password, &device_pub_key)
                    .map_err(|e| format!("Auto-login failed: {e}"))?;
                client = RelayClient::new(&client.base_url)
                    .with_session_token(session.session_token);
                let info = client.create_invite(&payload_base64, &expires_at_str)
                    .map_err(|e| e.to_string())?;
                Ok(info.url)
            }
            Err(e) => Err(e.to_string()),
        }
    }).await.map_err(|e| e.to_string())??;

    // 8. Persist relay URL
    {
        let mut mgrs = state.invite_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get_mut(&uuid).ok_or("No invite manager")?;
        mgr.set_relay_url(invite_id, relay_result.clone()).map_err(|e| e.to_string())?;
    }

    // 9. Build return value (manually set relay_url since record in step 3 didn't have it)
    let mut info = InviteInfo::from(record);
    info.relay_url = Some(relay_result);
    Ok(info)
}
```

- [ ] **Step 2: Add necessary imports at the top of `sync.rs`**

Check existing imports and add any missing ones: `base64`, `InviteManager`, `InviteInfo` from invites module, `RelayClient`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p krillnotes-desktop`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat(invite): implement share_invite_link command"
```

---

### Task 8: Implement `create_relay_invite` (replace stub)

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs:229-237`

- [ ] **Step 1: Replace the stub with real implementation**

This uploads an already-created invite to relay. Follows the `save_invite_file` pattern in `commands/invites.rs` for reconstructing the `InviteFile`:

```rust
#[tauri::command]
pub async fn create_relay_invite(
    window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    invite_id: String,
) -> Result<String, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let inv_id = Uuid::parse_str(&invite_id).map_err(|e| e.to_string())?;

    // Get signing key + declared name
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // Get invite record
    let record = {
        let mgrs = state.invite_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get(&uuid).ok_or("No invite manager")?;
        mgr.get_invite(inv_id).map_err(|e| e.to_string())?
            .ok_or("Invite not found")?
    };

    if record.revoked {
        return Err("Cannot upload a revoked invite".into());
    }

    // Get workspace metadata
    let (ws_desc, ws_author, ws_org, ws_url, ws_license, ws_tags) = {
        let wss = state.workspaces.lock().expect("Mutex poisoned");
        let ws = wss.get(window.label()).ok_or("No workspace open")?;
        let meta = ws.get_workspace_metadata().map_err(|e| e.to_string())?;
        (meta.description, meta.author_name, meta.author_org,
         meta.homepage_url, meta.license, meta.tags)
    };

    // Rebuild InviteFile (same pattern as save_invite_file command)
    let inviter_public_key = base64::engine::general_purpose::STANDARD
        .encode(signing_key.verifying_key().as_bytes());
    let invite_file = krillnotes_core::core::invite::InviteFile {
        file_type: "krillnotes-invite-v1".into(),
        invite_id: record.invite_id.to_string(),
        workspace_id: record.workspace_id.clone(),
        workspace_name: record.workspace_name.clone(),
        workspace_description: ws_desc,
        workspace_author_name: ws_author,
        workspace_author_org: ws_org,
        workspace_homepage_url: ws_url,
        workspace_license: ws_license,
        workspace_language: None,
        workspace_tags: ws_tags,
        inviter_public_key,
        inviter_declared_name: declared_name,
        expires_at: record.expires_at.map(|d| d.to_rfc3339()),
        signature: String::new(),
    };

    // Sign and serialize (sign_payload returns String, not Result)
    let signed_file = {
        let val = serde_json::to_value(&invite_file).map_err(|e| e.to_string())?;
        let sig = krillnotes_core::core::invite::sign_payload(&val, &signing_key);
        let mut f = invite_file;
        f.signature = sig;
        f
    };

    let bytes = InviteManager::serialize_invite_to_bytes(&signed_file)
        .map_err(|e| e.to_string())?;
    let payload_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let expires_at_str = record.expires_at
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| (chrono::Utc::now() + chrono::Duration::days(90)).to_rfc3339());

    // Get relay account
    let (relay_url_base, email, password, session_token, device_pub_key) = {
        let mgrs = state.relay_account_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get(&uuid).ok_or("No relay accounts for identity")?;
        let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
        let acct = accounts.first().ok_or("No relay account configured")?;
        (acct.relay_url.clone(), acct.email.clone(), acct.password.clone(),
         acct.session_token.clone(), acct.device_public_key.clone())
    };

    // Upload to relay
    let url = tokio::task::spawn_blocking(move || {
        let mut client = RelayClient::new(&relay_url_base)
            .with_session_token(session_token);
        match client.create_invite(&payload_base64, &expires_at_str) {
            Ok(info) => Ok(info.url),
            Err(krillnotes_core::KrillnotesError::RelayAuthExpired(_)) => {
                let session = client.login(&email, &password, &device_pub_key)
                    .map_err(|e| format!("Auto-login failed: {e}"))?;
                client = RelayClient::new(&client.base_url)
                    .with_session_token(session.session_token);
                let info = client.create_invite(&payload_base64, &expires_at_str)
                    .map_err(|e| e.to_string())?;
                Ok(info.url)
            }
            Err(e) => Err(e.to_string()),
        }
    }).await.map_err(|e| e.to_string())??;

    // Persist relay URL
    {
        let mut mgrs = state.invite_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get_mut(&uuid).ok_or("No invite manager")?;
        mgr.set_relay_url(inv_id, url.clone()).map_err(|e| e.to_string())?;
    }

    Ok(url)
}
```

Note: Check if `sign_payload` is `pub` in `invite.rs`. If it's `pub(crate)` or private, it may need to be made public, or reconstruct the invite via `InviteManager` methods. Adapt based on actual visibility.

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p krillnotes-desktop`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat(invite): implement create_relay_invite command (was stub)"
```

---

### Task 9: Implement `fetch_relay_invite` (replace stub)

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs:246-254`

- [ ] **Step 1: Replace the stub**

```rust
#[tauri::command]
pub async fn fetch_relay_invite(
    _window: Window,
    _state: State<'_, AppState>,
    token: String,
    relay_base_url: Option<String>,
) -> Result<crate::commands::invites::FetchedRelayInvite, String> {
    let base_url = relay_base_url.unwrap_or_else(|| "https://swarm.krillnotes.org".into());

    let (invite_file, raw_bytes) = tokio::task::spawn_blocking(move || {
        // No auth needed — GET /invites/{token} is public
        let client = RelayClient::new(&base_url);
        let payload = client.fetch_invite(&token)
            .map_err(|e| e.to_string())?;

        // Decode base64 payload to raw bytes
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&payload.payload)
            .map_err(|e| format!("Invalid base64 payload: {e}"))?;

        // Parse and verify
        let invite = InviteManager::parse_and_verify_invite_bytes(&bytes)
            .map_err(|e| e.to_string())?;

        Ok::<_, String>((invite, bytes))
    }).await.map_err(|e| e.to_string())??;

    // Write to temp file so respond_to_invite can use the path
    let temp_path = std::env::temp_dir().join(format!("kn-invite-{}.swarm", &invite_file.invite_id));
    std::fs::write(&temp_path, &raw_bytes)
        .map_err(|e| format!("Failed to write temp file: {e}"))?;

    // Convert to InviteFileData DTO
    use crate::commands::invites::{InviteFileData, FetchedRelayInvite};
    let fingerprint = krillnotes_core::core::contact::generate_fingerprint(&invite_file.inviter_public_key)
        .map_err(|e| e.to_string())?;

    let data = InviteFileData {
        invite_id: invite_file.invite_id,
        workspace_id: invite_file.workspace_id,
        workspace_name: invite_file.workspace_name,
        workspace_description: invite_file.workspace_description,
        workspace_author_name: invite_file.workspace_author_name,
        workspace_author_org: invite_file.workspace_author_org,
        workspace_homepage_url: invite_file.workspace_homepage_url,
        workspace_license: invite_file.workspace_license,
        workspace_language: invite_file.workspace_language,
        workspace_tags: invite_file.workspace_tags,
        inviter_public_key: invite_file.inviter_public_key,
        inviter_declared_name: invite_file.inviter_declared_name,
        inviter_fingerprint: fingerprint,
        expires_at: invite_file.expires_at,
    };

    Ok(FetchedRelayInvite {
        invite: data,
        temp_path: temp_path.to_string_lossy().to_string(),
    })
}
```

Note: Check how `fingerprint_of` is actually named/exported in the identity module. It may be named differently — search for `fingerprint` in `identity.rs` and adapt.

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p krillnotes-desktop`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat(invite): implement fetch_relay_invite command (was stub)"
```

---

### Task 10: Implement `send_invite_response_via_relay`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs`

- [ ] **Step 1: Add the command**

```rust
#[tauri::command]
pub async fn send_invite_response_via_relay(
    _window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    temp_path: String,
    expires_in_days: Option<u32>,
) -> Result<String, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Get signing key + declared name
    let (signing_key, declared_name) = {
        let ids = state.unlocked_identities.lock().expect("Mutex poisoned");
        let id = ids.get(&uuid).ok_or("Identity not unlocked")?;
        (
            crate::Ed25519SigningKey::from_bytes(&id.signing_key.to_bytes()),
            id.display_name.clone(),
        )
    };

    // Parse the invite from temp file
    let invite_file = InviteManager::parse_and_verify_invite(std::path::Path::new(&temp_path))
        .map_err(|e| e.to_string())?;

    // Build response
    let response = InviteManager::build_response(&invite_file, &signing_key, &declared_name)
        .map_err(|e| e.to_string())?;

    // Serialize response to bytes
    let bytes = InviteManager::serialize_response_to_bytes(&response)
        .map_err(|e| e.to_string())?;
    let payload_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    // Compute expiry
    let days = expires_in_days.unwrap_or(7);
    let expires_at_str = (chrono::Utc::now() + chrono::Duration::days(days as i64)).to_rfc3339();

    // Get relay account
    let (relay_url, email, password, session_token, device_pub_key) = {
        let mgrs = state.relay_account_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get(&uuid).ok_or("No relay accounts for identity")?;
        let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
        let acct = accounts.first().ok_or("No relay account configured")?;
        (acct.relay_url.clone(), acct.email.clone(), acct.password.clone(),
         acct.session_token.clone(), acct.device_public_key.clone())
    };

    // Upload response as relay invite
    let url = tokio::task::spawn_blocking(move || {
        let mut client = RelayClient::new(&relay_url)
            .with_session_token(session_token);
        match client.create_invite(&payload_base64, &expires_at_str) {
            Ok(info) => Ok(info.url),
            Err(krillnotes_core::KrillnotesError::RelayAuthExpired(_)) => {
                let session = client.login(&email, &password, &device_pub_key)
                    .map_err(|e| format!("Auto-login failed: {e}"))?;
                client = RelayClient::new(&client.base_url)
                    .with_session_token(session.session_token);
                let info = client.create_invite(&payload_base64, &expires_at_str)
                    .map_err(|e| e.to_string())?;
                Ok(info.url)
            }
            Err(e) => Err(e.to_string()),
        }
    }).await.map_err(|e| e.to_string())??;

    Ok(url)
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p krillnotes-desktop`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat(invite): add send_invite_response_via_relay command"
```

---

### Task 11: Implement `fetch_relay_invite_response`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs`

- [ ] **Step 1: Add the command**

```rust
#[tauri::command]
pub async fn fetch_relay_invite_response(
    _window: Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    token: String,
    relay_base_url: Option<String>,
) -> Result<crate::commands::invites::PendingPeer, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let base_url = relay_base_url.unwrap_or_else(|| "https://swarm.krillnotes.org".into());

    // Fetch from relay (no auth needed)
    let response_file = tokio::task::spawn_blocking(move || {
        let client = RelayClient::new(&base_url);
        let payload = client.fetch_invite(&token)
            .map_err(|e| e.to_string())?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&payload.payload)
            .map_err(|e| format!("Invalid base64: {e}"))?;
        InviteManager::parse_and_verify_response_bytes(&bytes)
            .map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())??;

    // Validate: check that the invite exists and is active
    {
        let mgrs = state.invite_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get(&uuid).ok_or("No invite manager")?;
        let inv_id = Uuid::parse_str(&response_file.invite_id).map_err(|e| e.to_string())?;
        let record = mgr.get_invite(inv_id).map_err(|e| e.to_string())?
            .ok_or("Invite not found — this response doesn't match any of your invites")?;
        if record.revoked {
            return Err("This invite has been revoked".into());
        }
        if let Some(exp) = record.expires_at {
            if exp < chrono::Utc::now() {
                return Err("This invite has expired".into());
            }
        }
    }

    // Increment use count
    {
        let mut mgrs = state.invite_managers.lock().expect("Mutex poisoned");
        let mgr = mgrs.get_mut(&uuid).ok_or("No invite manager")?;
        let inv_id = Uuid::parse_str(&response_file.invite_id).map_err(|e| e.to_string())?;
        mgr.increment_use_count(inv_id).map_err(|e| e.to_string())?;
    }

    // Build PendingPeer
    use crate::commands::invites::PendingPeer;
    let fingerprint = krillnotes_core::core::contact::generate_fingerprint(&response_file.invitee_public_key)
        .map_err(|e| e.to_string())?;

    Ok(PendingPeer {
        invite_id: response_file.invite_id,
        invitee_public_key: response_file.invitee_public_key,
        invitee_declared_name: response_file.invitee_declared_name,
        fingerprint,
    })
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p krillnotes-desktop`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat(invite): add fetch_relay_invite_response command"
```

---

### Task 12: Register new commands and remove stubs

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (generate_handler!)
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs` (remove stubs)

- [ ] **Step 1: Remove `parse_invite_bytes` and `write_temp_swarm_bytes` stubs from `sync.rs`**

Delete the stub functions (approximately lines 284-306 in sync.rs). These are replaced by `fetch_relay_invite` which does parsing and temp file writing internally.

- [ ] **Step 2: Update `generate_handler!` in `lib.rs`**

Check the existing import/registration pattern in `lib.rs`. Commands are currently registered as bare names (e.g., `create_relay_invite`, not `commands::sync::create_relay_invite`), with `use` imports at the top. Follow that same pattern.

Add imports for new commands and register them:
- `share_invite_link`
- `send_invite_response_via_relay`
- `fetch_relay_invite_response`

Remove deleted stubs from both imports and handler:
- `parse_invite_bytes`
- `write_temp_swarm_bytes`

The existing `create_relay_invite` and `fetch_relay_invite` entries stay (same names, new signatures).

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p krillnotes-desktop`
Expected: PASS

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat(invite): register new relay commands, remove parse/write stubs"
```

---

## Chunk 3: Frontend

### Task 13: Update TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:270-278`

- [ ] **Step 1: Add `relayUrl` to `InviteInfo`**

```typescript
export interface InviteInfo {
  inviteId: string;
  workspaceId: string;
  workspaceName: string;
  createdAt: string;
  expiresAt: string | null;
  revoked: boolean;
  useCount: number;
  relayUrl: string | null;
}
```

- [ ] **Step 2: Add `FetchedRelayInvite` interface**

```typescript
export interface FetchedRelayInvite {
  invite: InviteFileData;
  tempPath: string;
}
```

- [ ] **Step 3: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: Errors in components that use `InviteInfo` without `relayUrl` — that's fine, we'll fix those next.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(invite): add relayUrl to InviteInfo, add FetchedRelayInvite type"
```

---

### Task 14: Add "Share Invite Link" button to WorkspacePeersDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

- [ ] **Step 1: Add state and handler**

Add state variables near existing state declarations:

```typescript
const [sharingLink, setSharingLink] = useState(false);
const [shareError, setShareError] = useState<string | null>(null);
const [shareSuccess, setShareSuccess] = useState<string | null>(null);
const [showRelaySetup, setShowRelaySetup] = useState(false);
const [pendingShareAction, setPendingShareAction] = useState(false);
```

Add the handler function:

```typescript
const handleShareInviteLink = useCallback(async () => {
  setSharingLink(true);
  setShareError(null);
  setShareSuccess(null);
  try {
    // Check relay credentials first
    const hasRelay = await invoke<boolean>('has_relay_credentials');
    if (!hasRelay) {
      setPendingShareAction(true);
      setShowRelaySetup(true);
      setSharingLink(false);
      return;
    }
    await doShareInviteLink();
  } catch (e) {
    setShareError(String(e));
    setSharingLink(false);
  }
}, [identityUuid, workspaceName]);

const doShareInviteLink = useCallback(async () => {
  setSharingLink(true);
  try {
    const result = await invoke<InviteInfo>('share_invite_link', {
      identityUuid,
      workspaceName,
      expiresInDays: 7,  // TODO: use configurable default
    });
    if (result.relayUrl) {
      await navigator.clipboard.writeText(result.relayUrl);
      setShareSuccess(t('invite.linkCopied'));
    }
  } catch (e) {
    setShareError(String(e));
  } finally {
    setSharingLink(false);
  }
}, [identityUuid, workspaceName, t]);
```

- [ ] **Step 2: Add button to the dialog UI**

Place near the "Manage Invites" button (around line 405):

```tsx
<button
  onClick={handleShareInviteLink}
  disabled={sharingLink}
  className="..."  // match existing button styling
>
  {sharingLink ? t('invite.sharing') : t('invite.shareInviteLink')}
</button>
{shareError && <p className="text-red-500 text-sm mt-1">{shareError}</p>}
{shareSuccess && <p className="text-green-500 text-sm mt-1">{shareSuccess}</p>}
```

- [ ] **Step 3: Add relay account fallback dialog**

Near the existing dialog renders (InviteManagerDialog, etc.):

```tsx
{showRelaySetup && (
  <AddRelayAccountDialog
    identityUuid={identityUuid}
    onClose={() => {
      setShowRelaySetup(false);
      setPendingShareAction(false);
    }}
    onSuccess={() => {
      setShowRelaySetup(false);
      if (pendingShareAction) {
        setPendingShareAction(false);
        doShareInviteLink();
      }
    }}
  />
)}
```

- [ ] **Step 4: Add i18n keys**

Add to `en.json` (and other locale files):

```json
"invite.shareInviteLink": "Share Invite Link",
"invite.sharing": "Sharing...",
"invite.linkCopied": "Invite link copied to clipboard!",
"invite.importResponseFromLink": "Import Response from Link"
```

- [ ] **Step 5: Type-check and verify**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS (or pre-existing errors only)

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspacePeersDialog.tsx krillnotes-desktop/src/i18n/locales/
git commit -m "feat(invite): add Share Invite Link button to WorkspacePeersDialog"
```

---

### Task 15: Update InviteManagerDialog with relay features

**Files:**
- Modify: `krillnotes-desktop/src/components/InviteManagerDialog.tsx`

- [ ] **Step 1: Add "Share Invite Link" button at top**

Same pattern as WorkspacePeersDialog — add state, handler, button. Reuse the same `handleShareInviteLink` / `doShareInviteLink` / relay fallback pattern.

- [ ] **Step 2: Add relay URL display + Copy Link in invite list rows**

In the invite list rendering, for each invite:

```tsx
{invite.relayUrl ? (
  <div className="flex items-center gap-2">
    <span className="text-xs text-gray-500 truncate max-w-[200px]">{invite.relayUrl}</span>
    <button
      onClick={async () => {
        await navigator.clipboard.writeText(invite.relayUrl!);
        // Show brief "Copied!" feedback
      }}
      title={t('invite.copyLink')}
      className="..."
    >
      {/* Copy icon */}
    </button>
  </div>
) : !invite.revoked && (
  <button
    onClick={() => handleUploadToRelay(invite.inviteId)}
    className="..."
  >
    {t('invite.uploadToRelay')}
  </button>
)}
```

- [ ] **Step 3: Add `handleUploadToRelay` handler**

```typescript
const handleUploadToRelay = useCallback(async (inviteId: string) => {
  try {
    const hasRelay = await invoke<boolean>('has_relay_credentials');
    if (!hasRelay) {
      // Trigger relay setup fallback, store inviteId as pending
      setPendingUploadInviteId(inviteId);
      setShowRelaySetup(true);
      return;
    }
    const url = await invoke<string>('create_relay_invite', {
      identityUuid,
      inviteId,
    });
    // Refresh invite list to show the new URL
    await load();
  } catch (e) {
    setError(String(e));
  }
}, [identityUuid, load]);
```

- [ ] **Step 4: Add "Import Response from Link" section**

Add a button + text input for pasting a relay response URL:

```tsx
<div className="flex gap-2 mt-2">
  <input
    type="text"
    placeholder={t('invite.pasteResponseUrl')}
    value={responseUrl}
    onChange={(e) => setResponseUrl(e.target.value)}
    className="..."
  />
  <button
    onClick={handleFetchRelayResponse}
    disabled={!responseUrl.trim()}
  >
    {t('invite.fetchResponse')}
  </button>
</div>
```

Handler:

```typescript
const handleFetchRelayResponse = useCallback(async () => {
  try {
    // Extract token from URL (last path segment)
    const token = responseUrl.trim().split('/').pop() || '';
    const peer = await invoke<PendingPeer>('fetch_relay_invite_response', {
      identityUuid,
      token,
    });
    setPendingPeer(peer);
    setShowAcceptPeer(true);
  } catch (e) {
    setError(String(e));
  }
}, [responseUrl, identityUuid]);
```

- [ ] **Step 5: Add i18n keys**

```json
"invite.copyLink": "Copy Link",
"invite.uploadToRelay": "Upload to Relay",
"invite.pasteResponseUrl": "Paste response link URL...",
"invite.fetchResponse": "Fetch Response"
```

- [ ] **Step 6: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src/components/InviteManagerDialog.tsx krillnotes-desktop/src/i18n/locales/
git commit -m "feat(invite): add relay URL display and response import to InviteManagerDialog"
```

---

### Task 16: Update ImportInviteDialog for relay fetch + relay response

**Files:**
- Modify: `krillnotes-desktop/src/components/ImportInviteDialog.tsx`

- [ ] **Step 1: Update `handleFetchRelay` to use new `fetch_relay_invite`**

Replace the existing multi-call chain (lines ~63-82) with:

```typescript
const handleFetchRelay = useCallback(async () => {
  setFetchingRelay(true);
  setError(null);
  try {
    const token = extractRelayToken(relayUrl);
    if (!token) {
      setError(t('invite.invalidRelayUrl'));
      return;
    }
    const result = await invoke<FetchedRelayInvite>('fetch_relay_invite', { token });
    setInviteData(result.invite);
    setTempPath(result.tempPath);
```

Note: `extractRelayToken` already exists in the current `ImportInviteDialog.tsx` (around line 53-61). It extracts the last path segment from the URL. Reuse the existing function — do not reimplement it.

```typescript
  } catch (e) {
    setError(String(e));
  } finally {
    setFetchingRelay(false);
  }
}, [relayUrl, t]);
```

Add `tempPath` state:
```typescript
const [tempPath, setTempPath] = useState<string | null>(null);
```

- [ ] **Step 2: Add "Send via Relay" option to respond flow**

Update the respond section to offer two buttons:

```tsx
<div className="flex gap-2">
  <button
    onClick={handleRespondViaRelay}
    className="... primary styling"
  >
    {t('invite.sendViaRelay')}
  </button>
  <button
    onClick={handleRespondViaFile}
    className="... secondary styling"
  >
    {t('invite.saveResponseFile')}
  </button>
</div>
```

- [ ] **Step 3: Add `handleRespondViaRelay` handler**

```typescript
const handleRespondViaRelay = useCallback(async () => {
  if (!tempPath && !invitePath) return;
  setResponding(true);
  setError(null);
  try {
    const hasRelay = await invoke<boolean>('has_relay_credentials');
    if (!hasRelay) {
      setPendingRelayResponse(true);
      setShowRelaySetup(true);
      setResponding(false);
      return;
    }
    const url = await invoke<string>('send_invite_response_via_relay', {
      identityUuid: selectedIdentity,
      tempPath: tempPath || invitePath,
    });
    await navigator.clipboard.writeText(url);
    setResponseUrl(url);
    setStep('response_shared');  // new step showing success + URL
  } catch (e) {
    setError(String(e));
  } finally {
    setResponding(false);
  }
}, [tempPath, invitePath, selectedIdentity]);
```

- [ ] **Step 4: Add success state showing the response URL**

After successful relay response, show:

```tsx
{step === 'response_shared' && (
  <div>
    <p className="text-green-600">{t('invite.responseShared')}</p>
    <p className="text-sm text-gray-500 mt-1">{t('invite.shareResponseUrlWithInviter')}</p>
    <div className="flex gap-2 mt-2">
      <input readOnly value={responseUrl} className="..." />
      <button onClick={() => navigator.clipboard.writeText(responseUrl!)}>
        {t('invite.copyLink')}
      </button>
    </div>
  </div>
)}
```

- [ ] **Step 5: Add relay account fallback dialog**

Same pattern as WorkspacePeersDialog — `showRelaySetup` state, `AddRelayAccountDialog`, auto-continue on success.

- [ ] **Step 6: Add i18n keys**

```json
"invite.sendViaRelay": "Send via Relay",
"invite.saveResponseFile": "Save Response File",
"invite.responseShared": "Response link copied to clipboard!",
"invite.shareResponseUrlWithInviter": "Share this URL with the inviter to complete the connection.",
"invite.invalidRelayUrl": "Invalid relay invite URL"
```

- [ ] **Step 7: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add krillnotes-desktop/src/components/ImportInviteDialog.tsx krillnotes-desktop/src/i18n/locales/
git commit -m "feat(invite): wire relay fetch and relay response in ImportInviteDialog"
```

---

### Task 17: Update CreateInviteDialog call sites

> **Ordering note:** This task MUST be done in the same build as Task 8, since Task 8 changes the `create_relay_invite` parameter names. If the backend compiles with the new signature but the frontend still sends `{token}`, runtime errors will occur.

**Files:**
- Modify: `krillnotes-desktop/src/components/CreateInviteDialog.tsx`

- [ ] **Step 1: Update `handleCopyLink` to use new signature**

Replace the current call at ~line 83:

```typescript
// Old: invoke<string>('create_relay_invite', { token: createdInvite.inviteId })
// New:
const url = await invoke<string>('create_relay_invite', {
  identityUuid,
  inviteId: createdInvite.inviteId,
});
```

Make sure `identityUuid` is available as a prop or from context.

- [ ] **Step 2: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/CreateInviteDialog.tsx
git commit -m "fix(invite): update CreateInviteDialog to use new create_relay_invite signature"
```

---

### Task 18: Add i18n translations for all new keys

**Files:**
- Modify: all locale files in `krillnotes-desktop/src/i18n/locales/`

- [ ] **Step 1: List all locale files**

Use `Glob` to find `*.json` in `krillnotes-desktop/src/i18n/locales/`.

- [ ] **Step 2: Add all new keys to each locale file**

For `en.json`, add the keys from Tasks 14-16. For other languages, add the same keys with English values as placeholders (or translate if the language is known).

New keys:
```json
"invite.shareInviteLink": "Share Invite Link",
"invite.sharing": "Sharing...",
"invite.linkCopied": "Invite link copied to clipboard!",
"invite.copyLink": "Copy Link",
"invite.uploadToRelay": "Upload to Relay",
"invite.pasteResponseUrl": "Paste response link URL...",
"invite.fetchResponse": "Fetch Response",
"invite.importResponseFromLink": "Import Response from Link",
"invite.sendViaRelay": "Send via Relay",
"invite.saveResponseFile": "Save Response File",
"invite.responseShared": "Response link copied to clipboard!",
"invite.shareResponseUrlWithInviter": "Share this URL with the inviter to complete the connection.",
"invite.invalidRelayUrl": "Invalid relay invite URL"
```

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/
git commit -m "feat(i18n): add relay invite translation keys for all locales"
```

---

### Task 19: Full integration test

- [ ] **Step 1: Run Rust tests**

Run: `cargo test -p krillnotes-core`
Expected: All PASS

- [ ] **Step 2: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: PASS

- [ ] **Step 3: Build the app**

Run: `cd krillnotes-desktop && npm update && npm run tauri dev`
Expected: App launches without errors

- [ ] **Step 4: Manual test — inviter flow**

1. Open Workspace Peers dialog
2. Click "Share Invite Link"
3. If no relay account → verify AddRelayAccountDialog opens
4. After relay setup → verify invite is created and URL is copied
5. Open Manage Invites → verify the new invite shows the relay URL
6. Click "Copy Link" on the invite → verify URL is copied

- [ ] **Step 5: Manual test — invitee flow**

1. Open Import Invite dialog
2. Paste the relay invite URL
3. Click "Fetch" → verify invite details appear
4. Select identity, verify fingerprint
5. Click "Send via Relay" → verify response URL is copied
6. Share response URL with inviter

- [ ] **Step 6: Manual test — inviter accepts response**

1. Open Manage Invites
2. Paste response URL in "Import Response from Link"
3. Click "Fetch Response" → verify AcceptPeerDialog opens
4. Accept peer → verify peer appears in Workspace Peers

- [ ] **Step 7: Commit any remaining changes**

Stage only the specific files that were modified during testing/fixes. Do not use `git add -A`.

---

## Deferred: Default Expiry Setting

The spec calls for a per-identity `_settings.json` file storing `default_invite_expiry_days`, with a UI control in InviteManagerDialog. This is deferred to a follow-up PR to keep this one focused. For now, the default is hardcoded to 7 days in `share_invite_link` and `send_invite_response_via_relay`.

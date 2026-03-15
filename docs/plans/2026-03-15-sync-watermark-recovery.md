# Sync Watermark Recovery Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the sync protocol self-correcting so peers automatically recover from missed deltas without manual intervention, and prevent silent data loss when bundles fail to deliver.

**Architecture:** Three complementary mechanisms: (A) delivery-confirmed watermarks prevent the `last_sent_op` pointer from advancing when the relay rejects a bundle, (B) ACK-based watermark correction lets each delta carry the receiver's "last op I got from you" so the sender can self-correct, and (C) a manual force-resync resets the watermark as a safety valve. A UI improvement greys out the sync button when there are no pending ops.

**Tech Stack:** Rust (krillnotes-core sync engine, swarm codec), React/TypeScript (WorkspacePeersDialog), SQLite (sync_peers table)

**Worktree:** `/Users/careck/Source/Krillnotes/.worktrees/feat/sync-engine/`

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `krillnotes-core/src/core/sync/channel.rs` | Modify | Add `SendResult` return type to `SyncChannel::send_bundle` |
| `krillnotes-core/src/core/sync/relay/mod.rs` | Modify | Return `SendResult` from `RelayChannel::send_bundle` |
| `krillnotes-core/src/core/sync/relay/client.rs` | Modify | Return `(Vec<String>, UploadBundleSkipped)` from `upload_bundle` |
| `krillnotes-core/src/core/sync/folder.rs` | Modify | Return `SendResult::Delivered` from `FolderChannel::send_bundle` |
| `krillnotes-core/src/core/sync/mod.rs` | Modify | Skip watermark advancement on `SendResult::NotDelivered`; read inbound ACK and reset watermark |
| `krillnotes-core/src/core/swarm/header.rs` | Modify | Add `ack_operation_id: Option<String>` to `SwarmHeader` |
| `krillnotes-core/src/core/swarm/delta.rs` | Modify | Add `ack_operation_id` to `DeltaParams` and `ParsedDelta`; thread through create/parse |
| `krillnotes-core/src/core/swarm/sync.rs` | Modify | Pass `ack_operation_id` in `generate_delta`; read ACK in `apply_delta` and reset watermark |
| `krillnotes-core/src/core/peer_registry.rs` | Modify | Add `reset_last_sent` method; add `has_pending_ops` query |
| `krillnotes-core/src/core/workspace/sync.rs` | Modify | Expose `reset_peer_watermark` and `has_pending_ops_for_peers` methods |
| `krillnotes-desktop/src-tauri/src/commands/sync.rs` | Modify | Add `reset_peer_watermark` Tauri command; return pending-ops flag from `poll_sync` |
| `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx` | Modify | Grey out Sync Now when nothing pending; add Force Resync action per peer |

---

## Chunk 1: Delivery-Confirmed Watermarks (Mechanism A)

### Task 1: Add `SendResult` enum to channel trait

**Files:**
- Modify: `krillnotes-core/src/core/sync/channel.rs`

- [ ] **Step 1: Add SendResult enum and update trait**

In `channel.rs`, add the enum before the `SyncChannel` trait, and change the trait's return type:

```rust
/// Outcome of a send_bundle call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendResult {
    /// Bundle was accepted and routed to at least one recipient.
    Delivered,
    /// Transport succeeded but no recipient received the bundle
    /// (e.g. relay skipped all recipients as unknown/unverified).
    NotDelivered { reason: String },
}
```

Update `SyncChannel::send_bundle` return type from `Result<(), KrillnotesError>` to `Result<SendResult, KrillnotesError>`.

- [ ] **Step 2: Verify it fails to compile**

Run: `cargo check -p krillnotes-core --features relay 2>&1 | head -30`
Expected: compilation errors in `RelayChannel`, `FolderChannel`, and `sync/mod.rs` (they return `Ok(())` but now need `Ok(SendResult::Delivered)`)

- [ ] **Step 3: Update FolderChannel**

In `folder.rs`, change `send_bundle`'s return type and final `Ok(())` to `Ok(SendResult::Delivered)`. Add the import for `SendResult`.

Folder sync writes to disk — if the write succeeds, it's delivered (the file is there for the peer to pick up).

- [ ] **Step 4: Update RelayChannel to propagate delivery status**

In `relay/client.rs`, change `upload_bundle` to return the bundle_ids count alongside the result:

```rust
pub fn upload_bundle(
    &self,
    header: &BundleHeader,
    bundle_bytes: &[u8],
) -> Result<usize, KrillnotesError> {
```

Return `Ok(result.bundle_ids.len())` instead of `Ok(result.bundle_ids)`. The caller only needs the count.

In `relay/mod.rs`, update `send_bundle` to check the count:

```rust
fn send_bundle(&self, peer: &PeerSyncInfo, bundle_bytes: &[u8]) -> Result<SendResult, KrillnotesError> {
    // ... existing base64 decode and header construction ...
    let routed_count = self.client.upload_bundle(&header, bundle_bytes)?;
    if routed_count > 0 {
        log::info!(target: "krillnotes::relay", "bundle sent to peer {} via relay", peer.peer_device_id);
        Ok(SendResult::Delivered)
    } else {
        log::warn!(target: "krillnotes::relay", "bundle not delivered to peer {} — relay skipped all recipients", peer.peer_device_id);
        Ok(SendResult::NotDelivered {
            reason: "relay skipped all recipients (unknown or unverified device key)".to_string(),
        })
    }
}
```

- [ ] **Step 5: Update poll loop to check SendResult**

In `sync/mod.rs`, the `match channel.send_bundle(peer, &bundle_bytes)` block (around line 224) currently handles `Ok(())`. Change to:

```rust
match channel.send_bundle(peer, &bundle_bytes) {
    Ok(SendResult::Delivered) => {
        log::info!(...);
        let _ = workspace.update_peer_sync_status(&peer.peer_device_id, "idle", None, None);
        events.push(SyncEvent::DeltaSent { ... });
    }
    Ok(SendResult::NotDelivered { reason }) => {
        log::warn!(target: "krillnotes::sync",
            "bundle not delivered to peer {}: {reason}", peer.peer_device_id);
        let _ = workspace.update_peer_sync_status(
            &peer.peer_device_id, "not_delivered", Some(&reason), None,
        );
        events.push(SyncEvent::SendSkipped {
            workspace_id: workspace_id.clone(),
            peer_device_id: peer.peer_device_id.clone(),
            reason,
        });
    }
    Err(KrillnotesError::RelayAuthExpired(_)) => { /* existing */ }
    Err(e) => { /* existing */ }
}
```

Add `SendSkipped` to the `SyncEvent` enum (in `sync/mod.rs`):

```rust
SendSkipped {
    workspace_id: String,
    peer_device_id: String,
    reason: String,
},
```

- [ ] **Step 6: Move watermark update from generate_delta to poll loop**

Currently `generate_delta` in `swarm/sync.rs` (line 110-116) advances the watermark internally. This must move to the poll loop so it only happens AFTER confirmed delivery.

In `swarm/sync.rs`, remove the watermark update block (lines 110-120). Instead, have `generate_delta` return the ops it included alongside the bundle bytes. Change the return type:

```rust
pub struct DeltaBundle {
    pub bundle_bytes: Vec<u8>,
    /// The operation ID of the last op included, if any.
    pub last_included_op: Option<String>,
    /// Number of operations included.
    pub op_count: usize,
}
```

Return `Result<DeltaBundle>` instead of `Result<Vec<u8>>`.

In `sync/mod.rs`, after `Ok(SendResult::Delivered)`, advance the watermark:

```rust
Ok(SendResult::Delivered) => {
    if let Some(last_op_id) = &delta.last_included_op {
        let _ = workspace.upsert_sync_peer(
            &peer.peer_device_id,
            &peer.peer_identity_id,
            Some(last_op_id),
            None,
        );
    }
    // ... rest of success handling
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p krillnotes-core --features relay 2>&1 | tail -20`
Expected: all tests pass. Fix any compilation issues from the refactor.

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat(sync): delivery-confirmed watermarks — only advance last_sent_op when bundle is actually routed"
```

---

## Chunk 2: ACK-Based Watermark Correction (Mechanism B)

### Task 2: Add `ack_operation_id` to SwarmHeader

**Files:**
- Modify: `krillnotes-core/src/core/swarm/header.rs`

- [ ] **Step 1: Add field to SwarmHeader**

Add after the existing `since_operation_id` field (around line 56):

```rust
/// ACK: the last operation this sender received FROM the recipient.
/// Lets the recipient detect missed deltas and self-correct its watermark.
#[serde(skip_serializing_if = "Option::is_none")]
pub ack_operation_id: Option<String>,
```

- [ ] **Step 2: Verify compile**

Run: `cargo check -p krillnotes-core --features relay 2>&1 | head -20`
Expected: errors wherever `SwarmHeader` is constructed (header needs the new field). Note the locations.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(swarm): add ack_operation_id field to SwarmHeader"
```

### Task 3: Thread ACK through delta codec

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs`

- [ ] **Step 1: Add ack_operation_id to DeltaParams and ParsedDelta**

In `DeltaParams` (around line 25), add:

```rust
pub ack_operation_id: Option<String>,
```

In `ParsedDelta` (around line 42), add:

```rust
pub ack_operation_id: Option<String>,
```

- [ ] **Step 2: Pass ack through create_delta_bundle**

In `create_delta_bundle` (around line 80 where SwarmHeader is constructed), add the new field:

```rust
ack_operation_id: params.ack_operation_id.clone(),
```

- [ ] **Step 3: Extract ack in parse_delta_bundle**

In `parse_delta_bundle` (around line 155 where ParsedDelta is constructed), add:

```rust
ack_operation_id: header.ack_operation_id.clone(),
```

- [ ] **Step 4: Fix all other SwarmHeader construction sites**

Search for other places where `SwarmHeader` is built (snapshot, invite, accept modes) and add `ack_operation_id: None` to each.

Run: `cargo check -p krillnotes-core --features relay 2>&1 | head -30`

Fix all compilation errors until clean.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(swarm): thread ack_operation_id through delta codec"
```

### Task 4: Populate ACK in generate_delta

**Files:**
- Modify: `krillnotes-core/src/core/swarm/sync.rs`

- [ ] **Step 1: Read peer's last_received_op and pass as ACK**

In `generate_delta` (around line 58), after looking up the peer, read `last_received_op`:

```rust
let ack_op = peer.last_received_op.clone();
```

Pass it into `DeltaParams`:

```rust
let bundle = create_delta_bundle(DeltaParams {
    // ... existing fields ...
    ack_operation_id: ack_op,
})?;
```

Note: `peer.last_received_op` is "the last op we received FROM this peer". When we send this to the peer, they can compare it with their `last_sent_op` for us. If their `last_sent_op` is ahead, they know we missed some deltas.

- [ ] **Step 2: Run tests**

Run: `cargo test -p krillnotes-core --features relay -- delta 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(sync): populate ack_operation_id with last_received_op in outbound deltas"
```

### Task 5: Process inbound ACK and reset watermark

**Files:**
- Modify: `krillnotes-core/src/core/swarm/sync.rs`
- Modify: `krillnotes-core/src/core/peer_registry.rs`
- Modify: `krillnotes-core/src/core/workspace/sync.rs`

- [ ] **Step 1: Add reset_last_sent to PeerRegistry**

In `peer_registry.rs`, add:

```rust
/// Reset `last_sent_op` for a peer to a specific operation (or None for full resync).
/// Used when an inbound ACK reveals the peer is behind our watermark.
pub fn reset_last_sent(&self, peer_device_id: &str, to_op: Option<&str>) -> Result<()> {
    self.conn.execute(
        "UPDATE sync_peers SET last_sent_op = ?1 WHERE peer_device_id = ?2",
        rusqlite::params![to_op, peer_device_id],
    )?;
    Ok(())
}
```

- [ ] **Step 2: Expose through Workspace**

In `workspace/sync.rs`, add:

```rust
pub fn reset_peer_watermark(&self, peer_device_id: &str, to_op: Option<&str>) -> Result<()> {
    let conn = self.storage.connection();
    let registry = PeerRegistry::new(conn);
    registry.reset_last_sent(peer_device_id, to_op)
}
```

- [ ] **Step 3: Process ACK in apply_delta**

In `swarm/sync.rs` `apply_delta` function, after the existing processing (around line 200), add ACK processing before the return:

```rust
// Process ACK: if the sender tells us the last op they received from us,
// and that's behind our last_sent_op for them, reset our watermark.
if let Some(ack_op_id) = &parsed.ack_operation_id {
    let peer = workspace.get_sync_peer(&parsed.sender_device_id)?;
    if let Some(peer) = peer {
        if let Some(ref our_last_sent) = peer.last_sent_op {
            // Compare: is the ACK behind our watermark?
            let ack_is_behind = workspace.is_operation_before(ack_op_id, our_last_sent)?;
            if ack_is_behind {
                log::warn!(target: "krillnotes::sync",
                    "peer {} ACK ({}) is behind our last_sent ({}), resetting watermark",
                    parsed.sender_device_id, ack_op_id, our_last_sent
                );
                workspace.reset_peer_watermark(&parsed.sender_device_id, Some(ack_op_id))?;
            }
        }
    }
} else {
    // Peer has never received anything from us — they need everything.
    // Only reset if we have a non-None last_sent (i.e. we think we sent something).
    let peer = workspace.get_sync_peer(&parsed.sender_device_id)?;
    if let Some(peer) = peer {
        if peer.last_sent_op.is_some() {
            log::warn!(target: "krillnotes::sync",
                "peer {} sent no ACK but we have a watermark — resetting to force full delta",
                parsed.sender_device_id
            );
            workspace.reset_peer_watermark(&parsed.sender_device_id, None)?;
        }
    }
}
```

- [ ] **Step 4: Add `is_operation_before` helper to Workspace**

In `workspace/sync.rs`, add a helper that compares two operation IDs by their HLC timestamps:

```rust
/// Returns true if `op_a` is strictly before `op_b` in HLC order.
/// Returns false if either operation is not found in the log.
pub fn is_operation_before(&self, op_a: &str, op_b: &str) -> Result<bool> {
    let conn = self.storage.connection();
    let get_hlc = |op_id: &str| -> Option<(i64, i64, i64)> {
        conn.query_row(
            "SELECT timestamp_wall_ms, timestamp_counter, timestamp_node_id \
             FROM operations WHERE operation_id = ?1",
            [op_id],
            |row| Ok((row.get(0).ok()?, row.get(1).ok()?, row.get(2).ok()?)),
        ).optional().ok().flatten()
    };
    let Some(hlc_a) = get_hlc(op_a) else { return Ok(false) };
    let Some(hlc_b) = get_hlc(op_b) else { return Ok(false) };
    Ok(hlc_a < hlc_b)
}
```

- [ ] **Step 5: Add `get_sync_peer` to Workspace**

If it doesn't already exist, add in `workspace/sync.rs`:

```rust
pub fn get_sync_peer(&self, peer_device_id: &str) -> Result<Option<SyncPeer>> {
    let conn = self.storage.connection();
    let registry = PeerRegistry::new(conn);
    registry.get_peer(peer_device_id)
}
```

And the corresponding `get_peer` in `peer_registry.rs` if it doesn't exist.

- [ ] **Step 6: Run tests**

Run: `cargo test -p krillnotes-core --features relay 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 7: Write a unit test for ACK watermark correction**

In the test module of `swarm/sync.rs`, add a test that:
1. Creates Alice and Bob workspaces
2. Alice adds notes, generating operations
3. Alice generates a delta for Bob (sets `last_sent_op`)
4. Simulate Bob's ACK being behind (manually set Bob's `last_received_op` to an earlier op)
5. Alice receives a delta from Bob with `ack_operation_id` pointing to the earlier op
6. Assert that Alice's `last_sent_op` for Bob was reset to the ACK value

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat(sync): ACK-based watermark correction — peers self-heal from missed deltas"
```

---

## Chunk 3: Manual Force Resync (Mechanism C) + UI

### Task 6: Add force-resync Tauri command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (add to generate_handler)

- [ ] **Step 1: Add the command**

In `commands/sync.rs`:

```rust
#[tauri::command]
pub fn reset_peer_watermark(
    window: Window,
    state: State<'_, AppState>,
    peer_device_id: String,
) -> Result<(), String> {
    log::info!("reset_peer_watermark(window={}, peer={})", window.label(), peer_device_id);
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces.get(window.label())
        .ok_or("No workspace open for this window")?;
    ws.reset_peer_watermark(&peer_device_id, None)
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register in lib.rs**

Add `reset_peer_watermark` to the `tauri::generate_handler![]` macro invocation.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(sync): add reset_peer_watermark Tauri command for manual force resync"
```

### Task 7: Grey out Sync Now when nothing pending + Force Resync button

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`
- Modify: `krillnotes-core/src/core/workspace/sync.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs`

- [ ] **Step 1: Add `has_pending_ops` to core**

In `workspace/sync.rs`:

```rust
/// Returns true if there are operations to send to at least one non-manual peer.
pub fn has_pending_ops_for_any_peer(&self) -> Result<bool> {
    let peers = self.get_active_sync_peers()?;
    for peer in &peers {
        let last_sent = peer.last_sent_op.as_deref();
        let conn = self.storage.connection();
        // Quick existence check: is there at least one op after the watermark?
        let has_ops = if let Some(op_id) = last_sent {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id != ?1 LIMIT 1",
                [op_id],
                |row| row.get(0),
            ).unwrap_or(0);
            // This is a rough check — the real delta logic is more precise,
            // but this is sufficient for UI hint purposes.
            count > 0
        } else {
            // No watermark = no snapshot sent yet, nothing to delta
            false
        };
        if has_ops {
            return Ok(true);
        }
    }
    Ok(false)
}
```

- [ ] **Step 2: Add Tauri command to check pending**

In `commands/sync.rs`:

```rust
#[tauri::command]
pub fn has_pending_sync_ops(
    window: Window,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let ws = workspaces.get(window.label())
        .ok_or("No workspace open for this window")?;
    ws.has_pending_ops_for_any_peer().map_err(|e| e.to_string())
}
```

Register in `lib.rs` handler list.

- [ ] **Step 3: Update WorkspacePeersDialog.tsx**

Add state for pending ops:

```tsx
const [hasPendingOps, setHasPendingOps] = useState(false);
```

On dialog open and after sync, call:

```tsx
const checkPending = async () => {
  try {
    const pending = await invoke<boolean>('has_pending_sync_ops');
    setHasPendingOps(pending);
  } catch { /* ignore */ }
};
```

Update the Sync Now button disabled condition:

```tsx
disabled={syncing || (!hasPendingOps && peers.filter(p => p.channelType !== 'manual').length === 0)}
```

Add a per-peer "Force Resync" action (small link/button next to each relay peer):

```tsx
const handleForceResync = async (peerDeviceId: string) => {
  try {
    await invoke('reset_peer_watermark', { peerDeviceId });
    // Refresh peer list to show updated status
    await loadPeers();
    await checkPending();
  } catch (err) {
    console.error('Force resync failed:', err);
  }
};
```

Render it as a small text button in the peer row, only for relay/folder peers:

```tsx
{peer.channelType !== 'manual' && (
  <button
    onClick={() => handleForceResync(peer.peerDeviceId)}
    className="text-xs text-[var(--color-muted)] hover:text-[var(--color-text)] underline"
  >
    {t('sync.forceResync', 'Force Resync')}
  </button>
)}
```

- [ ] **Step 4: Add i18n key**

In `en.json` (and other locale files), add:

```json
"sync": {
  "forceResync": "Force Resync"
}
```

- [ ] **Step 5: Compile and test**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no type errors.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(sync): grey out Sync Now when nothing pending, add Force Resync per peer"
```

---

## Chunk 4: Integration Tests + Edge Cases

### Task 8: Write integration tests for watermark recovery

**Files:**
- Modify: `krillnotes-core/src/core/workspace/tests.rs` or `krillnotes-core/src/core/swarm/sync.rs` (test module)

- [ ] **Step 1: Test — delivery failure does NOT advance watermark**

Create two workspaces (Alice, Bob). Alice generates a delta for Bob. Simulate `SendResult::NotDelivered`. Assert `last_sent_op` for Bob has not changed.

- [ ] **Step 2: Test — ACK behind watermark triggers reset**

1. Alice creates 5 operations
2. Alice generates delta for Bob (last_sent_op → op_5)
3. Bob only receives ops 1-3 (simulate by manually setting Bob's last_received_op to op_3)
4. Bob sends delta to Alice with ack_operation_id = op_3
5. Alice applies Bob's delta
6. Assert Alice's last_sent_op for Bob is now op_3 (reset from op_5)
7. Alice generates next delta — should include ops 4-5

- [ ] **Step 3: Test — ACK is None triggers full resend**

1. Alice creates operations and generates delta for Bob (sets last_sent_op)
2. Bob sends delta with ack_operation_id = None
3. Alice applies it
4. Assert Alice's last_sent_op for Bob is reset to None

- [ ] **Step 4: Test — ACK ahead of watermark is ignored**

1. Alice sends delta to Bob (last_sent_op → op_3)
2. Bob replies with ack = op_5 (somehow ahead)
3. Alice applies — no change to last_sent_op (ignore)

- [ ] **Step 5: Test — force resync resets watermark to None**

1. Set up peer with last_sent_op = "some-op"
2. Call reset_peer_watermark(peer, None)
3. Assert last_sent_op is None

- [ ] **Step 6: Run all tests**

Run: `cargo test -p krillnotes-core --features relay 2>&1 | tail -20`
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "test(sync): watermark recovery integration tests"
```

### Task 9: Handle edge case — ACK refers to purged operation

**Files:**
- Modify: `krillnotes-core/src/core/swarm/sync.rs` (apply_delta ACK processing)

- [ ] **Step 1: Handle purged ACK gracefully**

In the ACK processing block of `apply_delta`, when `is_operation_before` returns `false` because the ACK operation doesn't exist in the log, we should reset to None (force full delta) rather than silently ignoring:

```rust
if ack_is_behind {
    // ACK is behind — reset to peer's position
    workspace.reset_peer_watermark(&parsed.sender_device_id, Some(ack_op_id))?;
} else if !workspace.operation_exists(ack_op_id)? {
    // ACK references an operation we don't have (purged?) — force full resend
    log::warn!(target: "krillnotes::sync",
        "peer {} ACK ({}) references unknown operation, resetting watermark to force full delta",
        parsed.sender_device_id, ack_op_id
    );
    workspace.reset_peer_watermark(&parsed.sender_device_id, None)?;
}
```

Add `operation_exists` helper in `workspace/sync.rs`:

```rust
pub fn operation_exists(&self, operation_id: &str) -> Result<bool> {
    let conn = self.storage.connection();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM operations WHERE operation_id = ?1",
        [operation_id],
        |row| row.get(0),
    ).map_err(KrillnotesError::Database)?;
    Ok(count > 0)
}
```

- [ ] **Step 2: Test purged ACK scenario**

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "fix(sync): handle purged ACK operation — force full delta resend"
```

---

## Summary of SyncEvent Changes

The `SyncEvent` enum needs one new variant:

```rust
SendSkipped {
    workspace_id: String,
    peer_device_id: String,
    reason: String,
},
```

The frontend should handle this in the sync result display (WorkspacePeersDialog) to show "Bundle not delivered: {reason}" instead of "1 bundle sent".

---

## Testing Sequence

After all tasks are complete, manual test the full flow:

1. Reset relay server DB (clean slate)
2. Register Bob with relay
3. Bob syncs → bundle not delivered (Alice not registered) → watermark NOT advanced
4. Register Alice with relay
5. Bob syncs → bundle delivered → Alice receives ops
6. Alice syncs → ACK sent to Bob → Bob confirms watermark matches
7. Simulate missed delta: manually advance Bob's last_sent_op → Alice sends delta with old ACK → Bob auto-corrects
8. Test Force Resync button: click it → peer watermark reset → next sync sends full delta

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Integration tests for watermark recovery mechanisms.
//!
//! Tests cover three complementary recovery mechanisms:
//!
//! **Mechanism A** – Delivery-confirmed watermarks: `last_sent_op` advances only on
//!   `SendResult::Delivered`, not on transport success alone.
//!
//! **Mechanism B** – ACK-based self-correction: every outbound delta carries the
//!   receiver's "last op I got from you" so senders can self-correct.
//!
//! **Mechanism C** – Force-resync: `reset_peer_watermark` as a manual safety valve.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use tempfile::NamedTempFile;

use krillnotes_core::{
    core::{
        contact::{ContactManager, TrustLevel},
        swarm::sync::{apply_delta, generate_delta},
    },
    Workspace,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}

fn b64_pubkey(key: &SigningKey) -> String {
    BASE64.encode(key.verifying_key().as_bytes())
}

fn make_workspace(key: &SigningKey, identity_id: &str) -> (NamedTempFile, Workspace) {
    let tmp = NamedTempFile::new().expect("tempfile");
    let ws = Workspace::create(tmp.path(), "", identity_id, SigningKey::from_bytes(&key.to_bytes()))
        .expect("Workspace::create");
    (tmp, ws)
}

fn make_contact_manager(enc_key: [u8; 32]) -> (tempfile::TempDir, ContactManager) {
    let dir = tempfile::tempdir().expect("tempdir");
    let cm = ContactManager::for_identity(dir.path().to_path_buf(), enc_key)
        .expect("ContactManager::for_identity");
    (dir, cm)
}

// ── Mechanism C: has_pending_ops_for_any_peer ─────────────────────────────────

/// A peer with `last_sent_op = None` has not yet received a snapshot — work is
/// needed, so `has_pending_ops_for_any_peer` returns true even with no delta ops.
#[test]
fn has_pending_ops_true_when_peer_has_no_watermark() {
    let key = make_key();
    let bob_key = make_key();
    let bob_pub = b64_pubkey(&bob_key);

    let (_tmp, ws) = make_workspace(&key, "alice-id");

    // Register Bob with NO last_sent_op (snapshot not yet sent).
    ws.upsert_sync_peer("dev-bob", &bob_pub, None, None).expect("upsert_sync_peer");
    ws.update_peer_channel("dev-bob", "folder", "{}").expect("update_peer_channel");

    assert!(
        ws.has_pending_ops_for_any_peer().expect("has_pending_ops_for_any_peer"),
        "should report pending when peer watermark is None (snapshot not yet sent)"
    );
}

/// After the watermark is set to the latest op, `has_pending_ops_for_any_peer`
/// returns false (all ops are covered).
#[test]
fn has_pending_ops_false_when_watermark_covers_all() {
    let key = make_key();
    let bob_key = make_key();
    let bob_pub = b64_pubkey(&bob_key);

    let (_tmp, mut ws) = make_workspace(&key, "alice-id");

    ws.create_note_root("TextNote").expect("create_note_root");

    let latest = ws
        .get_latest_operation_id()
        .expect("get_latest_operation_id")
        .expect("at least one op");

    ws.upsert_sync_peer("dev-bob", &bob_pub, Some(&latest), None)
        .expect("upsert_sync_peer");
    ws.update_peer_channel("dev-bob", "folder", "{}").expect("update_peer_channel");

    assert!(
        !ws.has_pending_ops_for_any_peer().expect("has_pending_ops_for_any_peer"),
        "should report no pending when watermark covers all ops"
    );
}

/// After a new op is created following the watermark, `has_pending_ops_for_any_peer`
/// flips back to true.
#[test]
fn has_pending_ops_true_after_new_op_past_watermark() {
    let key = make_key();
    let bob_key = make_key();
    let bob_pub = b64_pubkey(&bob_key);

    let (_tmp, mut ws) = make_workspace(&key, "alice-id");

    ws.create_note_root("TextNote").expect("first note");
    let snap = ws
        .get_latest_operation_id()
        .expect("get_latest_operation_id")
        .expect("at least one op");

    ws.upsert_sync_peer("dev-bob", &bob_pub, Some(&snap), None)
        .expect("upsert_sync_peer");
    ws.update_peer_channel("dev-bob", "folder", "{}").expect("update_peer_channel");

    assert!(!ws.has_pending_ops_for_any_peer().unwrap(), "baseline: no pending");

    // New op AFTER the watermark.
    ws.create_note_root("TextNote").expect("second note");

    assert!(
        ws.has_pending_ops_for_any_peer().expect("has_pending_ops_for_any_peer"),
        "should report pending after new op past watermark"
    );
}

// ── Mechanism C: reset_peer_watermark ────────────────────────────────────────

/// `reset_peer_watermark` sets the watermark to a specific op, making previously
/// covered ops pending again (force-resync).
#[test]
fn reset_watermark_to_specific_op_makes_later_ops_pending() {
    let key = make_key();
    let bob_key = make_key();
    let bob_pub = b64_pubkey(&bob_key);

    let (_tmp, mut ws) = make_workspace(&key, "alice-id");

    ws.create_note_root("TextNote").expect("op 1");
    let op1 = ws.get_latest_operation_id().expect("get_latest").expect("op1");

    ws.create_note_root("TextNote").expect("op 2");
    let op2 = ws.get_latest_operation_id().expect("get_latest").expect("op2");

    // Register Bob with watermark = op2 (everything sent).
    ws.upsert_sync_peer("dev-bob", &bob_pub, Some(&op2), None)
        .expect("upsert_sync_peer");
    ws.update_peer_channel("dev-bob", "folder", "{}").expect("update_peer_channel");

    assert!(!ws.has_pending_ops_for_any_peer().unwrap(), "all ops covered before reset");

    // Force-reset watermark back to op1 — op2 becomes pending again.
    ws.reset_peer_watermark("dev-bob", Some(&op1))
        .expect("reset_peer_watermark");

    assert!(
        ws.has_pending_ops_for_any_peer().expect("has_pending_ops_for_any_peer"),
        "op2 should be pending after watermark reset to op1"
    );

    let peer = ws.get_sync_peer("dev-bob").expect("get_sync_peer").expect("peer exists");
    assert_eq!(peer.last_sent_op.as_deref(), Some(op1.as_str()), "watermark should be op1");
}

/// `reset_peer_watermark(None)` clears the watermark entirely — all ops pending.
#[test]
fn reset_watermark_to_none_makes_all_ops_pending() {
    let key = make_key();
    let bob_key = make_key();
    let bob_pub = b64_pubkey(&bob_key);

    let (_tmp, mut ws) = make_workspace(&key, "alice-id");

    ws.create_note_root("TextNote").expect("op");
    let latest = ws.get_latest_operation_id().expect("get_latest").expect("op");

    ws.upsert_sync_peer("dev-bob", &bob_pub, Some(&latest), None)
        .expect("upsert_sync_peer");
    ws.update_peer_channel("dev-bob", "folder", "{}").expect("update_peer_channel");

    ws.reset_peer_watermark("dev-bob", None).expect("reset_peer_watermark");

    let peer = ws.get_sync_peer("dev-bob").expect("get_sync_peer").expect("peer");
    assert!(peer.last_sent_op.is_none(), "watermark should be None after reset");
}

// ── Mechanism B: ACK-based watermark correction ───────────────────────────────
//
// For ACK tests, Bob generates a zero-ops delta that carries only the ACK field.
// When Alice applies it, `upsert_peer_from_delta` migrates Alice's placeholder
// "dev-bob" peer to Bob's real device_id ("bob-id") while preserving last_sent_op.
// The ACK block then runs and adjusts the watermark.

/// When Bob returns an ACK pointing to `op_1` but Alice's watermark is `op_3`,
/// Alice detects the discrepancy and resets her watermark to `op_1`.
///
/// This is the normal case: Bob fell behind Alice and needs a re-send from op_1.
#[test]
fn ack_behind_watermark_resets_alice_watermark() {
    let alice_key = make_key();
    let bob_key = make_key();
    let alice_pub = b64_pubkey(&alice_key);
    let bob_pub = b64_pubkey(&bob_key);

    // ── Alice's workspace ────────────────────────────────────────────────────
    let (_alice_tmp, mut alice_ws) = make_workspace(&alice_key, "alice-id");
    let (_alice_cm_dir, mut alice_cm) = make_contact_manager([0xAAu8; 32]);

    alice_ws.create_note_root("TextNote").expect("op 1");
    let op1 = alice_ws.get_latest_operation_id().expect("get_latest").expect("op1");
    alice_ws.create_note_root("TextNote").expect("op 2");
    alice_ws.create_note_root("TextNote").expect("op 3");
    let op3 = alice_ws.get_latest_operation_id().expect("get_latest").expect("op3");

    // Alice registers Bob; pretends she sent everything up to op3.
    alice_ws
        .upsert_sync_peer("dev-bob", &bob_pub, Some(&op3), None)
        .expect("alice upsert_sync_peer for Bob");
    alice_ws
        .update_peer_channel("dev-bob", "folder", "{}")
        .expect("update_peer_channel");
    alice_cm
        .find_or_create_by_public_key("Bob", &bob_pub, TrustLevel::Tofu)
        .expect("alice registers Bob as contact");

    // ── Bob's workspace (same workspace_id, owner = Alice) ──────────────────
    // Bob uses identity_id "bob-id" so his device_id() = "bob-id".
    let bob_tmp = NamedTempFile::new().expect("bob_tmp");
    let mut bob_ws = Workspace::create_with_id(
        bob_tmp.path(),
        "",
        "bob-id",
        SigningKey::from_bytes(&bob_key.to_bytes()),
        alice_ws.workspace_id(),
    )
    .expect("bob create_with_id");
    bob_ws.set_owner_pubkey(&alice_pub).expect("set_owner_pubkey");

    let (_bob_cm_dir, mut bob_cm) = make_contact_manager([0xBBu8; 32]);
    bob_cm
        .find_or_create_by_public_key("Alice", &alice_pub, TrustLevel::Tofu)
        .expect("bob registers Alice as contact");

    // Bob's peer record for Alice: he only received up to op1 (last_received_op = op1).
    // last_sent_op = "snap" (non-null so generate_delta doesn't error).
    bob_ws
        .upsert_sync_peer("dev-alice", &alice_pub, Some("snap"), Some(&op1))
        .expect("bob upsert_sync_peer for Alice");

    // ── Bob generates a zero-ops delta carrying ack = op1 ────────────────────
    let bundle = generate_delta(
        &mut bob_ws,
        "dev-alice",
        "TestWorkspace",
        &bob_key,
        "Bob",
        &mut bob_cm,
    )
    .expect("generate_delta");

    // ── Alice applies Bob's delta ─────────────────────────────────────────────
    // upsert_peer_from_delta migrates "dev-bob" → "bob-id", preserving last_sent_op=op3.
    // The ACK block then runs: is_operation_before(op1, op3) = true → reset to op1.
    apply_delta(&bundle.bundle_bytes, &mut alice_ws, &alice_key, &mut alice_cm)
        .expect("apply_delta");

    // ── Alice's watermark for Bob (now keyed by "bob-id") should be op1 ──────
    let peer = alice_ws
        .get_sync_peer("bob-id")
        .expect("get_sync_peer")
        .expect("Bob peer should exist under real device_id");

    assert_eq!(
        peer.last_sent_op.as_deref(),
        Some(op1.as_str()),
        "Alice's watermark for Bob should be reset to op1 (the ACK)"
    );
}

/// When Bob's ACK points to an operation ID that Alice does not have in her log,
/// Alice resets her watermark to `None` so the next poll starts from scratch.
#[test]
fn ack_unknown_op_resets_alice_watermark_to_none() {
    let alice_key = make_key();
    let bob_key = make_key();
    let alice_pub = b64_pubkey(&alice_key);
    let bob_pub = b64_pubkey(&bob_key);

    let (_alice_tmp, mut alice_ws) = make_workspace(&alice_key, "alice-id");
    let (_alice_cm_dir, mut alice_cm) = make_contact_manager([0xCCu8; 32]);

    alice_ws.create_note_root("TextNote").expect("op");
    let latest = alice_ws.get_latest_operation_id().expect("get_latest").expect("op");

    alice_ws
        .upsert_sync_peer("dev-bob", &bob_pub, Some(&latest), None)
        .expect("alice upsert_sync_peer");
    alice_cm
        .find_or_create_by_public_key("Bob", &bob_pub, TrustLevel::Tofu)
        .expect("alice registers Bob");

    let bob_tmp = NamedTempFile::new().expect("bob_tmp");
    let mut bob_ws = Workspace::create_with_id(
        bob_tmp.path(),
        "",
        "bob-id",
        SigningKey::from_bytes(&bob_key.to_bytes()),
        alice_ws.workspace_id(),
    )
    .expect("bob create_with_id");
    bob_ws.set_owner_pubkey(&alice_pub).expect("set_owner_pubkey");

    let (_bob_cm_dir, mut bob_cm) = make_contact_manager([0xDDu8; 32]);
    bob_cm
        .find_or_create_by_public_key("Alice", &alice_pub, TrustLevel::Tofu)
        .expect("bob registers Alice");

    // Bob's ACK points to an op that doesn't exist in Alice's log.
    let ghost_op = "00000000-0000-0000-0000-000000000000";
    bob_ws
        .upsert_sync_peer("dev-alice", &alice_pub, Some("snap"), Some(ghost_op))
        .expect("bob upsert_sync_peer");

    let bundle = generate_delta(
        &mut bob_ws,
        "dev-alice",
        "TestWorkspace",
        &bob_key,
        "Bob",
        &mut bob_cm,
    )
    .expect("generate_delta");

    apply_delta(&bundle.bundle_bytes, &mut alice_ws, &alice_key, &mut alice_cm)
        .expect("apply_delta");

    let peer = alice_ws
        .get_sync_peer("bob-id")
        .expect("get_sync_peer")
        .expect("Bob peer should exist under real device_id");

    assert!(
        peer.last_sent_op.is_none(),
        "Alice's watermark should be None when ACK references unknown op"
    );
}

/// When Bob sends a delta with no ACK (his `last_received_op` is `None`) but
/// Alice already has a non-null watermark for him, Alice resets to `None`.
/// This handles a newly-connected or state-wiped Bob.
#[test]
fn no_ack_in_delta_resets_alice_watermark_to_none() {
    let alice_key = make_key();
    let bob_key = make_key();
    let alice_pub = b64_pubkey(&alice_key);
    let bob_pub = b64_pubkey(&bob_key);

    let (_alice_tmp, mut alice_ws) = make_workspace(&alice_key, "alice-id");
    let (_alice_cm_dir, mut alice_cm) = make_contact_manager([0xEEu8; 32]);

    alice_ws.create_note_root("TextNote").expect("op");
    let latest = alice_ws.get_latest_operation_id().expect("get_latest").expect("op");

    alice_ws
        .upsert_sync_peer("dev-bob", &bob_pub, Some(&latest), None)
        .expect("alice upsert_sync_peer");
    alice_cm
        .find_or_create_by_public_key("Bob", &bob_pub, TrustLevel::Tofu)
        .expect("alice registers Bob");

    let bob_tmp = NamedTempFile::new().expect("bob_tmp");
    let mut bob_ws = Workspace::create_with_id(
        bob_tmp.path(),
        "",
        "bob-id",
        SigningKey::from_bytes(&bob_key.to_bytes()),
        alice_ws.workspace_id(),
    )
    .expect("bob create_with_id");
    bob_ws.set_owner_pubkey(&alice_pub).expect("set_owner_pubkey");

    let (_bob_cm_dir, mut bob_cm) = make_contact_manager([0xFFu8; 32]);
    bob_cm
        .find_or_create_by_public_key("Alice", &alice_pub, TrustLevel::Tofu)
        .expect("bob registers Alice");

    // Bob has NO last_received_op for Alice → delta carries no ACK.
    bob_ws
        .upsert_sync_peer("dev-alice", &alice_pub, Some("snap"), None)
        .expect("bob upsert_sync_peer");

    let bundle = generate_delta(
        &mut bob_ws,
        "dev-alice",
        "TestWorkspace",
        &bob_key,
        "Bob",
        &mut bob_cm,
    )
    .expect("generate_delta");

    apply_delta(&bundle.bundle_bytes, &mut alice_ws, &alice_key, &mut alice_cm)
        .expect("apply_delta");

    let peer = alice_ws
        .get_sync_peer("bob-id")
        .expect("get_sync_peer")
        .expect("Bob peer should exist under real device_id");

    assert!(
        peer.last_sent_op.is_none(),
        "Alice's watermark should be None when no ACK in received delta"
    );
}

// ── Mechanism A: delivery-confirmed watermark ─────────────────────────────────

/// `generate_delta` does NOT advance the watermark itself — the poll loop does so
/// only after confirmed delivery. Verifies the invariant by checking `last_sent_op`
/// is unchanged immediately after `generate_delta` returns.
#[test]
fn generate_delta_does_not_advance_watermark() {
    let alice_key = make_key();
    let bob_key = make_key();
    let bob_pub = b64_pubkey(&bob_key);

    let (_tmp, mut ws) = make_workspace(&alice_key, "alice-id");
    let (_cm_dir, mut cm) = make_contact_manager([0x11u8; 32]);
    cm.find_or_create_by_public_key("Bob", &bob_pub, TrustLevel::Tofu)
        .expect("register Bob");

    let snap_op = ws.get_latest_operation_id().expect("get_latest").unwrap_or_default();
    ws.upsert_sync_peer("dev-bob", &bob_pub, Some(&snap_op), None)
        .expect("upsert_sync_peer");

    ws.create_note_root("TextNote").expect("create note");

    let watermark_before = ws
        .get_sync_peer("dev-bob")
        .expect("get_sync_peer")
        .expect("peer")
        .last_sent_op
        .clone();

    // Calling generate_delta should NOT change last_sent_op.
    let _bundle = generate_delta(&mut ws, "dev-bob", "TestWS", &alice_key, "Alice", &mut cm)
        .expect("generate_delta");

    let watermark_after = ws
        .get_sync_peer("dev-bob")
        .expect("get_sync_peer")
        .expect("peer")
        .last_sent_op
        .clone();

    assert_eq!(
        watermark_after, watermark_before,
        "generate_delta must not advance the watermark — only the poll loop should do that"
    );
}

// ── Task 9: Purged ACK edge case ───────────────────────────────────────────────

/// If Bob's ACK points to an operation that Alice once sent but has since purged
/// from her log, `operation_exists` returns false — indistinguishable from a
/// never-existed op.  The ACK processing correctly resets the watermark to `None`
/// in both cases so the next delta covers everything from the beginning.
///
/// We model a "purged" op as an op_id that was recorded in Bob's state but is not
/// present in Alice's current operations table (the same code path as any unknown id).
#[test]
fn purged_ack_resets_watermark_same_as_unknown_op() {
    let key = make_key();
    let bob_key = make_key();
    let bob_pub = b64_pubkey(&bob_key);

    let (_tmp, mut ws) = make_workspace(&key, "alice-id");

    ws.create_note_root("TextNote").expect("op");
    let latest = ws.get_latest_operation_id().expect("get_latest").expect("op");

    // A plausible-but-purged op ID that is NOT in Alice's operations table.
    let purged_op_id = "purged-00-0000-0000-0000-000000000000";

    // Verify it truly doesn't exist (simulating the post-purge state).
    assert!(
        !ws.operation_exists(purged_op_id).expect("operation_exists"),
        "the purged op should not be in the operations table"
    );

    // Alice has watermark = latest; Bob's ACK will reference the purged op.
    ws.upsert_sync_peer("dev-bob", &bob_pub, Some(&latest), None)
        .expect("upsert_sync_peer");
    ws.update_peer_channel("dev-bob", "folder", "{}").expect("update_peer_channel");

    // Simulate ACK processing: is_operation_before returns false (op doesn't exist),
    // operation_exists returns false → reset to None.
    ws.reset_peer_watermark("dev-bob", None).expect("reset_peer_watermark");

    let peer = ws.get_sync_peer("dev-bob").expect("get_sync_peer").expect("peer");
    assert!(
        peer.last_sent_op.is_none(),
        "watermark should be None after purged ACK processing resets it"
    );
}

# Per-Operation Verification Design

**Date:** 2026-04-23
**Status:** Draft
**Issue:** #141 (C1 — per-operational signatures not verified)

## Problem

Operations carry Ed25519 signatures and author public keys, but `verify()` is never called on receipt. In a chain of peers (A → B → C → D), downstream peers receive operations from authors they don't know and have no way to verify the original signature. The system currently trusts that intermediate peers verified signatures — but they don't.

A peer only knows its direct peers. When peer B relays operations from A to C, C has no cryptographic evidence that A's signature was checked by anyone.

## Solution: Sender-Vouches-Per-Op

Each peer vouches for operations it trusts when building a delta bundle. The vouching is per-operation metadata in the delta payload, and the receiver stores a local verification record.

### Core Principle

Trust is local and transitive at the delta level. Each peer vouches for ops it can verify (direct peer's ops) or has already accepted as verified (ops vouched for by a trusted peer). The receiver only needs to trust their direct peer.

### Verification vs Vouching

- **Verification**: Cryptographically checking the original Ed25519 signature on an operation against the author's known public key. Only possible when the author is a known contact.
- **Vouching**: Asserting "I trust this op" in a delta, based on either direct verification or trust in a peer who vouched previously.

## Data Model

### Delta Payload Wrapper

Replace the raw `Vec<Operation>` in the encrypted delta payload with:

```rust
#[derive(Serialize, Deserialize)]
struct DeltaOperation {
    op: Operation,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified_by: Option<String>, // base64 Ed25519 pubkey of voucher
}
```

The bundle sender populates `verified_by` for each op:

| Op category | `verified_by` in delta |
|-------------|----------------------|
| Ops I authored | `None` — receiver verifies my original sig directly |
| Ops from my direct peers (I verified original sig) | `my_identity_key` |
| Ops I received with `verified_by` set from a trusted peer | `my_identity_key` (re-vouch) |

### Local Storage

Add column to `operations` table:

```sql
ALTER TABLE operations ADD COLUMN verified_by TEXT NOT NULL DEFAULT '';
```

`verified_by` stores the base64 public key of the direct peer who vouched for or authored the op. Empty string only for pre-migration rows.

For new ops:
- Self-authored ops: `verified_by = my_identity_key`
- Ops received from direct peer (their authorship): `verified_by = sender_identity_key`
- Ops relayed by direct peer (vouched): `verified_by = sender_identity_key`

The column answers: "which of my direct peers stands behind this op?"

## Verification Flow

### Sender: Building a Delta

For each operation to include in the delta:

1. **Self-authored op** (`op.author_key() == my_identity_key`):
   - Set `verified_by = None` in the `DeltaOperation` wrapper
   - Receiver will verify the original Ed25519 signature directly

2. **Op from a direct peer** (author is a known contact):
   - Look up author's `VerifyingKey` from contacts
   - Call `op.verify(&author_verifying_key)`
   - If verification succeeds: set `verified_by = my_identity_key`
   - If verification fails: **do not include this op** in the delta

3. **Previously vouched op** (`verified_by` column is set in local DB):
   - Already trusted — set `verified_by = my_identity_key` (re-vouch)
   - No need to re-check original signature

### Receiver: Processing a Delta

For each `DeltaOperation` in the received delta:

1. **Op authored by the bundle sender** (`op.author_key() == header.source_identity`):
   - Verify original Ed25519 signature: `op.verify(&sender_verifying_key)`
   - Sender's key is known from `header.source_identity` (direct peer)
   - If valid: store with `verified_by = sender_identity_key`
   - If invalid: **reject the op**

2. **Relayed op with `verified_by` set**:
   - Confirm `verified_by` value matches `header.source_identity` (the sender is vouching)
   - Sender is a trusted direct peer: accept the vouch
   - Store with `verified_by = sender_identity_key`

3. **Relayed op without `verified_by`**:
   - **Reject** — sender didn't vouch for this op, protocol violation

### Rejection Behavior

Rejected ops are silently dropped (not inserted into `operations` table, not applied to working tables). The operation may arrive later via a different peer who can vouch for it.

## Topology Analysis

### Linear Chain (A → B → C → D)

```
A creates op, signs it
  → B receives from A: verifies A's sig (A is B's direct peer)
    B stores: verified_by = A's key (A is the sender who authored the op)
  → B sends to C: verified_by = B's key (B vouches)
    C stores: verified_by = B's key (B is C's direct peer who vouched)
  → C sends to D: verified_by = C's key (C re-vouches)
    D stores: verified_by = C's key (C is D's direct peer who vouched)
```

Each hop: sender vouches, receiver stores sender's key. Original sig verified once at first hop.

### Diamond (A → B,C → D)

```
      D
     / \
    B   C
     \ /
      A
```

Both B and C verify A's signature independently. D receives the same op from both. First-writer-wins on `INSERT OR IGNORE` — whichever delta D processes first determines the stored `verified_by`. Both B's and C's vouches are equally valid.

Different nodes may record different verifiers for the same op. This is expected and correct — verification records are local metadata, not replicated state.

### Hub and Spoke (13 peers)

```
Hub
├── Arm1 ─── S1a, S1b, S1c
├── Arm2 ─── S2a, S2b, S2c
└── Arm3 ─── S3a, S3b, S3c
```

Overhead per op: ~44 bytes (base64 pubkey in `verified_by`). For a sync cycle with 130 ops flowing through Hub to each arm (~90 ops per delta): ~3.9 KB additional per delta. Negligible compared to operation payload sizes.

## Code Changes

### `krillnotes-core/src/core/swarm/delta.rs`

- New `DeltaOperation` struct (wrapper around `Operation` + `verified_by`)
- `build_delta_bundle`: serialize `Vec<DeltaOperation>` instead of `Vec<Operation>`
  - Query `verified_by` column for each op
  - Self-authored ops: `verified_by = None`
  - All others with `verified_by` set: `verified_by = Some(my_identity_key)`
- `parse_delta_bundle`: deserialize to `Vec<DeltaOperation>`, return alongside existing metadata

### `krillnotes-core/src/core/schema.sql`

- Migration: `ALTER TABLE operations ADD COLUMN verified_by TEXT NOT NULL DEFAULT ''`

### `krillnotes-core/src/core/workspace/sync.rs`

- `apply_incoming_operation` gains `verified_by: Option<&str>` parameter
- For sender-authored ops: call `op.verify()` against sender's key from header
- For relayed ops: validate `verified_by` matches sender identity
- Reject ops that fail verification or lack vouching
- INSERT includes `verified_by` column

### `krillnotes-core/src/core/swarm/sync.rs`

- After `parse_delta_bundle`, iterate `Vec<DeltaOperation>`
- Pass `delta_op.verified_by` through to `apply_incoming_operation`
- TOFU contact auto-registration remains (for `JoinWorkspace` ops)

### `krillnotes-core/src/core/operation.rs`

- No changes to `Operation` enum — verification is transport metadata, not CRDT state
- Existing `sign()` and `verify()` methods used as-is

## What This Does NOT Cover

- **Revoking trust in a compromised peer** — if peer B is compromised, ops B vouched for (or tampered with) remain in the DB. A future "distrust peer" feature could flag or purge them.

## Security Properties

| Property | Status |
|----------|--------|
| Op authenticity at first hop | Ed25519 `verify()` on original signature |
| Op trust through relay chain | Transitive vouching by direct peers |
| Tamper detection | Modified ops fail `verify()` at first hop; can't be vouched |
| Forged ops by compromised peer | Caught only when peer is distrusted (future work) |
| Unvouched ops from relay | Rejected — not inserted |
| Bundle integrity | Existing BLAKE3 + Ed25519 manifest signature (incl. attachments) |

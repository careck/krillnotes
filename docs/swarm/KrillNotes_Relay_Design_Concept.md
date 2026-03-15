# KRILLNOTES RELAY

## Lightweight Store-and-Forward Sync Service

**Version 0.1 — Design Concept**

**Date: March 2026**

**Status: DRAFT — Conceptual Design**

**Companion to:** KrillNotes Swarm Design v0.7

**Companion to:** Swarm Server Design Proposal v0.1

---

## 1. The Problem

The Swarm protocol is transport-agnostic by design. A .swarm file can move between devices via USB drive, email attachment, shared folder, AirDrop, or any other mechanism that transfers a file from point A to point B. This is a powerful property — it means the protocol has zero infrastructure dependency and works in fully disconnected environments.

However, for everyday users who are within internet connectivity and want to share KrillNotes workspaces with colleagues, friends, or family, manually downloading, copying, and emailing .swarm files back and forth is too cumbersome. The same friction applies to syncing between a user's own devices — desktop and mobile running the same identity need a seamless way to exchange .swarm bundles without manual file transfers.

These users don't need the full weight of the OPswarm Base (enterprise command infrastructure with HSM-backed root keys, intelligence loops, compliance anchoring, and multi-incident coordination). They need something much simpler: a hosted mailbox that moves .swarm files between devices automatically.

---

## 2. The Core Idea

The KrillNotes Relay is an always-on, hosted mailbox service for .swarm files. It stores encrypted bundles addressed to specific recipients and delivers them when those recipients connect. It is a **transport adapter**, not a Swarm peer.

The Relay never decrypts .swarm payloads. It never participates in conflict resolution. It never holds root owner keys. It never merges operations. It doesn't know what a "note" is. From the Swarm protocol's perspective, the Relay is equivalent to a shared Dropbox folder — just a transport mechanism — but with user accounts, a proper API, and push delivery so the KrillNotes desktop app can automate the exchange.

> *Design principle: The Relay is a convenience layer over the transport-agnostic protocol. If the Relay goes offline, manual .swarm file exchange still works. Nothing breaks without it — sync just becomes manual again.*

---

## 3. What the Relay Is and Is Not

| The Relay IS | The Relay IS NOT |
|---|---|
| An opaque store-and-forward mailbox for .swarm files | A Swarm peer that participates in the protocol |
| A public hosting endpoint for .cloud broadcast bundles | An automated .cloud generation engine (that's Base) |
| A transport adapter alongside USB, email, shared folders | The only or primary way to sync |
| Authenticated — every interaction requires an account (except .cloud reads) | A decryption endpoint — it never sees plaintext |
| A convenience service for connected users | A dependency — the protocol works without it |
| Open source (MPL 2.0), self-hostable | A commercial product (that's OPswarm Base) |

---

## 4. Relationship to OPswarm Base

The Relay and OPswarm Base occupy fundamentally different positions in the architecture. The Relay is a **transport pipe** — it moves opaque .swarm files and hosts .cloud broadcasts. OPswarm Base is a **protocol peer** — it actively participates in workspaces, decrypts content, enforces RBAC, and runs automated intelligence pipelines.

| Capability | KrillNotes Relay | OPswarm Base |
|---|---|---|
| Opaque .swarm routing | Yes (key-based) | Yes (plus content-aware RBAC filtering) |
| User accounts | Yes (one identity per account) | Yes (institutional identities, HSM-backed) |
| Workspace mailboxes | Yes | Yes |
| .cloud broadcast hosting | Yes (manual publish) | Yes (automated generation from workspace state) |
| Multi-device sync | Yes | Yes |
| Protocol participation (decrypt, merge, RBAC) | No (opaque transport) | Yes (full peer) |
| Root authority management | No | Yes (HSM key vault) |
| Intelligence loop (VERSD) | No | Yes |
| Operational monitoring | No | Yes (sync freshness, clock drift, anomaly detection) |
| Compliance & audit anchoring | No | Yes |
| Multi-incident coordination | No | Yes |
| LoRa / satellite transport | No | Yes |

The natural upgrade path: a team that starts with the free Relay and outgrows it — because they need institutional root authority, automated monitoring, RBAC-aware routing, or the intelligence loop — steps up to OPswarm Base.

---

## 5. Account Model

### 5.1 One Account, One Identity

A Relay account binds exactly one email address to exactly one KrillNotes identity. The identity may have one or more device keys (see Section 7), each representing a device the user operates. The account holder authenticates to the Relay via email + password (or passkey/magic link). The Relay uses the registered device keys for routing — when a .swarm bundle lists one of these keys in its recipient list, the Relay delivers it to this account.

The Relay never receives or stores any private key. Authentication to the Relay (email/password) is entirely separate from the KrillNotes identity system (Ed25519 keypairs with passphrase-protected private keys). The Relay only needs the public device keys to match incoming bundles to accounts.

### 5.2 Why One Identity Per Account

KrillNotes supports multiple identities per user (e.g., "Carsten @ 2pi" for work, "Carsten K" for personal). The Relay intentionally limits each account to a single identity. Users with multiple identities need multiple Relay accounts — one per identity. Each identity's Relay account can be on a different Relay server (see Section 13.1), and the credentials for each are encrypted to that identity's key, inaccessible when the identity is locked.

This constraint keeps the Relay simple (no identity-switching logic, no multi-key routing), avoids potential for cross-identity metadata correlation on the server, and creates a natural limitation that OPswarm Base removes.

### 5.3 Account Recovery

The Relay supports standard password reset via email. If a user loses access to their email and cannot reset their password, the account is flagged for deletion after a configurable grace period (default: 90 days). During the grace period, pending bundles are retained but inaccessible. After the grace period, the account and all associated data (mailboxes, pending bundles) are permanently deleted.

Account recovery is entirely separate from KrillNotes identity recovery. The Relay does not hold identity keys; losing Relay access does not affect the user's local workspaces or their ability to sync via other channels.

### 5.4 Account Roles

Each account has a role: `user` or `admin`. The admin role grants access to Relay administration functions (account management, storage monitoring, configuration) when an admin interface is implemented. The initial version has no admin UI, but the role field is present in the data model from day one to avoid a schema migration later.

### 5.5 Public Key Verification (Proof of Possession)

When registering a device key with the Relay — whether during initial account creation or when adding a subsequent device — the Relay verifies that the registrant genuinely holds the private key corresponding to the public key they are registering. This prevents an attacker from registering someone else's device key under their own account and intercepting bundles meant for the genuine key holder.

The verification flow:

1. The user submits an Ed25519 public key (their device key) during registration or device addition.
2. The Relay generates a random nonce, encrypts it to the claimed public key (using X25519 key agreement, consistent with the Swarm protocol's encryption model), and sends the encrypted challenge to the client.
3. The user's KrillNotes app decrypts the nonce using the corresponding private key and returns the plaintext nonce.
4. The Relay verifies the nonce matches. If correct, the device key is registered against the account.

This proof-of-possession is performed once per device key. Combined with key-based bundle routing (Section 8.2), this gives a strong delivery guarantee: only the proven key holder can receive bundles encrypted for their device key.

### 5.6 Account Data

| Field | Purpose |
|---|---|
| `account_id` | Internal identifier (UUID) |
| `email` | Authentication and account recovery |
| `auth_credential` | Password hash (Argon2id) or passkey |
| `identity_uuid` | The identity this account represents |
| `device_keys[]` | One or more Ed25519 public keys, one per device (each verified via proof of possession) |
| `role` | `user` or `admin` |
| `created_at` | Account creation timestamp |
| `storage_used` | Total bytes of .swarm bundles currently held |
| `flagged_for_deletion` | Timestamp when deletion was flagged (null if active) |

---

## 6. Workspace Mailboxes

### 6.1 Concept

When a user participates in a workspace, their app registers a **mailbox** for that workspace on the Relay. The mailbox is simply a record saying "this account is a member of workspace X." When a .swarm bundle arrives for workspace X, the Relay queues it for every account that has a mailbox for that workspace (excluding the sender).

### 6.2 Mailbox Data

| Field | Purpose |
|---|---|
| `account_id` | The account that owns this mailbox |
| `workspace_id` | The workspace this mailbox receives bundles for |
| `registered_at` | When the mailbox was created |
| `pending_bundles` | Count of undelivered bundles |
| `storage_used` | Bytes of pending bundles for this mailbox |

### 6.3 Mailbox Lifecycle

A mailbox is created when the user joins a workspace (via invitation acceptance or workspace creation) and their app has a Relay connection configured. It is removed when the user leaves the workspace, or manually by the user via the app. The app manages mailbox registration automatically — the user doesn't interact with mailboxes directly.

---

## 7. Multi-Device Sync

### 7.1 The Problem with Shared Keys

The Swarm protocol's identity model defines an identity as an Ed25519 keypair. The peer registry tracks peers by public key, and bundle generation creates one delta per peer key. If two devices share the same keypair, the protocol sees one peer — and the "don't send to self" rule means neither device generates bundles for the other. Multi-device sync is impossible with a single shared key, regardless of the transport channel.

### 7.2 Per-Device Keys Linked to a Single Identity

Each device generates its own Ed25519 keypair. A user's desktop has `key_desktop`, their phone has `key_phone`. These are distinct keys linked to the same identity through a signed declaration — an `AddDeviceKey` operation where an existing device key signs a statement asserting that the new key also belongs to this identity.

In the workspace peer registry, both device keys appear as separate peers. Bundle generation works naturally: the desktop generates deltas for `key_phone` just as it does for Bob's key. The phone generates deltas for `key_desktop`. Every transport channel handles multi-device without special cases — the Relay routes by recipient key as designed, shared folders work, manual exchange works.

This is consistent with how other multi-device end-to-end encrypted systems (Signal, Matrix) handle the problem: per-device keys linked to a person-level identity.

### 7.3 Identity Model Evolution

The identity model evolves from "one keypair = one identity" to "one identity = a person who controls one or more device keys." An identity becomes a logical entity with:

- A stable identity UUID and display name (the "person")
- One or more device keys, each with its own Ed25519 keypair
- An `AddDeviceKey` operation linking a new device key to the identity (signed by an existing device key)
- A `RevokeDeviceKey` operation for removing a lost or compromised device (signed by any remaining device key)

RBAC evaluates against the identity, not the individual device key. When a peer is granted Writer on /Project Alpha, all device keys for that identity inherit the permission. Operation attribution resolves to the identity level — "Carsten" authored the edit, regardless of which device key signed it. The UI may optionally show "Carsten (desktop)" for disambiguation, but the permission and authorship model operates at the identity level.

### 7.4 Device Onboarding via .swarmid

The `.swarmid` file's role is to authorise a new device under an existing identity. The onboarding flow for a second device:

1. The user installs KrillNotes on the new device.
2. The new device generates a fresh Ed25519 keypair.
3. The user links the new device to their existing identity — by scanning a QR code displayed on the first device, or by opening a `.swarmid` authorisation file transferred from the first device.
4. The first device signs an `AddDeviceKey` operation declaring the new device's public key as belonging to this identity.
5. The `AddDeviceKey` operation propagates to all workspace peers through normal sync.
6. The new device's key now appears in the peer registry of every workspace the identity participates in.
7. Existing peers begin generating delta bundles for the new device key. The new device receives a snapshot (or accumulated deltas) to catch up.

For the first device (identity creation), the device key *is* the founding key of the identity. No `.swarmid` step is needed — the identity begins with one device.

### 7.5 Implications for the Relay Account Model

The one-account-one-identity constraint still holds, but the account now registers multiple device keys rather than a single public key. All device keys for the identity are verified via proof of possession (Section 5.5) — each device proves it holds the corresponding private key when it connects.

When a .swarm bundle lists `key_phone` as a recipient, the Relay delivers it to the account that has `key_phone` registered as one of its device keys. The routing logic (Section 8.2) remains key-based — the only change is that an account may have multiple registered keys instead of one.

### 7.6 Device Revocation

If a device is lost or compromised, any remaining device key for the identity can sign a `RevokeDeviceKey` operation. This propagates through normal sync, and peers stop generating bundles for the revoked key. On the Relay, the revoked key is removed from the account's registered device keys, and any pending bundles for that key are discarded.

> *Note: The per-device key model and the `AddDeviceKey`/`RevokeDeviceKey` operations represent a protocol-level change that requires a corresponding update to the Swarm Design Specification (v0.7, Section 14 — Identity Model). This document describes the design intent; the formal operation definitions belong in the protocol spec.*

---

## 8. Bundle Routing

### 8.1 Opaque Store-and-Forward

The Relay reads only the unencrypted `header.json` from each .swarm bundle. The header contains: workspace ID, sender device key, recipient device keys, bundle mode (invite/accept/snapshot/delta), and timestamp. The encrypted payload is stored and forwarded as an opaque blob.

### 8.2 Key-Based Routing

Bundle delivery is determined by recipient device keys, not workspace membership. When a bundle is uploaded:

1. Read `sender_device_key` and the list of `recipient_device_keys` from the header.
2. For each recipient device key, find the account that registered that key (via the verified proof-of-possession in Section 5.5).
3. Queue the bundle for each matching recipient key, excluding the sender's own device key (but *not* excluding other device keys on the same account — this is how multi-device sync works: your desktop's bundle addressed to your phone's key is delivered to the same account but a different device).
4. Notify connected recipients via WebSocket (or queue for next poll).
5. If a recipient device key has no matching account on this Relay, the bundle is simply not queued for that key — the sender's app will need to deliver via another channel.

This means the Relay only delivers bundles to accounts whose verified device key appears in the bundle's recipient list. An account that registers a mailbox for a workspace but whose device keys are never listed as recipients in any bundle will never receive anything. The workspace mailbox (Section 6) serves as a client-side organisational tool for the account holder, but the actual delivery decision is cryptographically grounded in the recipient key list.

### 8.3 Bundle Retention

Bundles are stored until the recipient downloads them, or until a configurable retention period expires (e.g., 30 days). Downloaded bundles are deleted from the Relay. The Relay is a transit point, not an archive.

### 8.4 Storage Accounting

Each account has a storage quota (total bytes of pending bundles across all mailboxes). This provides natural rate-limiting and a practical nudge toward OPswarm Base for high-volume workspaces.

### 8.5 Transitive Propagation and Indirect Peers

A workspace may contain peers who do not all have direct sync relationships with each other. If Alice syncs with Bob, and Bob syncs with Carol, but Alice and Carol do not sync directly — Carol is an **indirect peer** from Alice's perspective. Alice's operations still reach Carol, because when Bob applies Alice's delta, those operations enter Bob's operation log. When Bob next generates a delta for Carol, it includes all operations since their last sync — including Alice's.

This transitive propagation means workspaces naturally develop efficient topologies without requiring every peer to sync with every other peer. A well-connected peer (someone who is frequently online, or a team lead) naturally becomes a hub, reducing the total bundle count across the workspace. The Relay supports this transparently — it routes bundles based on recipient keys, and is unaware of the broader peer topology.

---

## 9. Per-Peer Channel Configuration

### 9.1 The Channel Concept

The Relay is one of several transport channels available in KrillNotes. Each peer relationship within a workspace is configured with a delivery channel that governs how .swarm bundles flow between those two peers. The channel is bidirectional — both peers use the same channel for a given workspace.

Available channel types:

| Channel | Description |
|---|---|
| `relay` | Bundles routed via the KrillNotes Relay. Requires both peers to have Relay accounts. |
| `folder` | Bundles written to and read from a shared folder (Dropbox, Syncthing, NAS, S3). |
| `manual` | Bundles saved to a local outbox for the user to deliver themselves (email, USB, AirDrop). |

### 9.2 Channel Is Per-Peer, Per-Workspace

The channel configuration lives on the **peer registry entry within the workspace**, alongside the existing `last_sent_op`, `last_received_op`, and `last_sync` fields. This means Bob might use the Relay for "Book Club Notes" but manual exchange for "Contract Review" — the sensitivity or context of the workspace drives the channel choice, not just the person's general preference.

### 9.3 Extended Peer Registry

| Field | Purpose |
|---|---|
| `peer_id` | The peer's public key |
| `display_name` | Human-readable name |
| `last_sent_op` | Last operation ID sent to this peer |
| `last_received_op` | Last operation ID received from this peer |
| `last_sync` | Timestamp of last sync exchange |
| `channel_type` | `relay`, `folder`, or `manual` |
| `channel_params` | Channel-specific configuration (JSON) |

Channel parameters by type:

- **relay:** `{ "relay_url": "https://relay.krillnotes.org", "peer_account_ref": "<public_key>" }`
- **folder:** `{ "path": "/shared/syncthing/bob-workspace-alpha" }`
- **manual:** `{}` (no parameters — bundles go to the local outbox)

### 9.4 Channel Declared During Invitation Acceptance

When a peer accepts a workspace invitation, their accept .swarm payload includes a channel preference declaration for this workspace. The inviter's app records this on the peer's registry entry. Both sides then use the declared channel for all subsequent sync in this workspace.

If the acceptor has a Relay account and wants to use it for this workspace, they declare `relay` with their Relay URL. If they prefer manual exchange, they declare `manual`. The choice is made per-workspace at invitation time, and can be changed later.

### 9.5 Channel Change Propagation

If a peer changes their channel preference for a workspace (e.g., Carol finally gets a Relay account and wants to switch from manual to relay), this is communicated via a signed channel-update message distributed through the *current* channel. The next manual .swarm exchange includes the channel update, and from that point forward both sides switch to the new channel.

---

## 10. The Sync Dispatch Loop

The KrillNotes desktop app's sync engine runs a simple dispatch loop:

1. For each workspace the user participates in:
2. For each peer in that workspace's peer registry:
3. If there are new operations since `last_sent_op`:
4. Generate a delta .swarm bundle for this peer.
5. Route the bundle via the peer's configured channel:
   - **relay:** POST to the Relay API.
   - **folder:** Write to the configured shared folder path.
   - **manual:** Save to the local outbox; optionally trigger the OS share sheet.
6. Update `last_sent_op` for this peer.

The inbound side mirrors this: the app listens on all active channels simultaneously. The Relay WebSocket delivers bundles from relay-connected peers. The folder watcher picks up bundles from shared-folder peers. Manual import handles files the user opens explicitly. All three feed into the same bundle ingestion pipeline. The app doesn't care how a .swarm arrived.

---

## 11. Invitation Flow via the Relay

### 11.1 Invite Distribution

Invitations in the current protocol are signed by the inviter's public key and can be sent to multiple recipients (the inviter doesn't need to know the recipients in advance). The invite is a broadcast token: "here's my workspace, here's my public key, here's the role I'm offering, come join if you want."

Distribution of the invite is always out-of-band — email, Slack, QR code, printed on a whiteboard. The Relay does not handle invite distribution because the inviter doesn't know who will accept.

### 11.2 Reply Channels in the Invite Payload

The invite payload includes a `reply_channels` field that tells the recipient how to send back their accept .swarm:

```json
"reply_channels": [
  {
    "type": "relay",
    "url": "https://relay.krillnotes.org/invites/{pairing_token}/accept"
  },
  {
    "type": "manual",
    "label": "Send me the .swarm file directly"
  }
]
```

The acceptor's app renders these as choices in the accept dialog. The Relay option is a one-click authenticated POST; the manual option saves the accept .swarm locally for the user to deliver themselves.

### 11.3 Relay Invite Endpoint

When the inviter generates an invite and has a Relay account, their app registers a short-lived **invite mailbox** on the Relay:

1. The app calls `POST /invites` with the pairing token and the invite's expiry timestamp.
2. The Relay creates a temporary mailbox keyed to the pairing token, held open until expiry.
3. The invite payload includes the Relay reply URL.

When a recipient accepts via the Relay:

1. The acceptor authenticates with their own Relay account.
2. The acceptor's app POSTs the accept .swarm to `POST /invites/{pairing_token}/accept`.
3. The Relay queues the accept .swarm for the inviter's account.
4. The inviter's app receives notification, downloads the accept, and completes the handshake.

Both the inviter and acceptor must have Relay accounts to use the Relay invite endpoint. This keeps the service fully authenticated with no anonymous upload vectors.

### 11.4 Link-Based Invitation Flow

The Relay invite endpoint enables a frictionless link-based onboarding experience:

1. The inviter generates an invite in KrillNotes. The app registers it on the Relay.
2. Instead of (or in addition to) sending a .swarm file, the inviter shares a URL: `https://relay.krillnotes.org/invites/{pairing_token}`
3. The recipient clicks the link. The Relay serves the invite payload.
4. If the recipient has KrillNotes installed, the app opens and displays the invitation.
5. If the recipient doesn't have KrillNotes, the page prompts installation.
6. If the recipient doesn't have a Relay account, the page prompts signup — the invitation itself becomes the onboarding trigger for the Relay service.
7. The recipient accepts in-app, their accept .swarm is POSTed back through the Relay, and the handshake completes.

The entire onboarding becomes: share a link, click accept. No .swarm files changed hands visibly. The protocol is identical underneath.

### 11.5 Expiry and Revocation

Invitations have a timed expiry embedded in the signed payload. The Relay's invite mailbox is automatically cleaned up at expiry. The inviter can also manually revoke an invite before expiry — the app sends a `DELETE /invites/{pairing_token}` to the Relay, and any subsequent accept attempts are rejected.

---

## 12. .cloud Broadcast Hosting

### 12.1 Concept

The .cloud format is the Swarm protocol's broadcast publication mode: cleartext-signed, one-way, no encryption. A .cloud bundle contains operations signed by the publisher's identity but readable by anyone. The Relay can host .cloud bundles as public endpoints, making them available for download without authentication.

This uses the same infrastructure pattern as the link-based invitation flow (Section 11.4): the Relay serves a signed payload at a public URL. The difference is that invitations are temporary (pairing-token-scoped, short-lived) while .cloud publications are ongoing (workspace-scoped, optionally long-lived).

### 12.2 Publication Flow

1. The user generates a .cloud bundle in KrillNotes (from a workspace where they are the root owner or have publication rights).
2. The app uploads the .cloud bundle to the Relay via `POST /broadcasts`.
3. The Relay hosts the bundle at a public URL: `https://relay.krillnotes.org/broadcasts/{workspace_id}` (or a custom slug).
4. The publisher shares the URL however they like — website, social media, email newsletter.
5. Anyone can download the .cloud bundle from that URL. No Relay account required for readers.
6. Subsequent .cloud delta bundles can be uploaded to the same endpoint, creating a feed of updates.

### 12.3 .cloud vs .swarm on the Relay

| Property | .swarm bundles | .cloud bundles |
|---|---|---|
| Encryption | Encrypted (per-recipient) | Cleartext (signed only) |
| Access | Authenticated (recipient account required) | Public (no account required for readers) |
| Routing | Key-based (delivered to specific accounts) | URL-based (available to anyone with the link) |
| Direction | Bidirectional (peers exchange bundles) | One-way (publisher → readers) |
| Authentication to publish | Account required | Account required |
| Authentication to read | Account required | None |

### 12.4 Expiry and Retention

.cloud publications can have an optional expiry timestamp. Expired publications are cleaned up automatically. The publisher can also manually remove a publication via `DELETE /broadcasts/{broadcast_id}`. Retention periods and storage limits for .cloud bundles are governed by the same configurable parameters as .swarm bundles (Section 18).

### 12.5 Relationship to OPswarm Base

The Relay hosts .cloud bundles that the user's app generates manually. OPswarm Base *generates* .cloud bundles automatically — after each merge cycle, the Base evaluates which operations affect public subtrees and produces a .cloud delta bundle without any user intervention. The distinction is manual publication (Relay) vs. automated publication pipeline (Base).

---

## 13. Desktop App Integration

### 13.1 Relay Setup

A Relay connection is configured **per identity**, not globally for the application. KrillNotes supports multiple identities, and more than one can be unlocked simultaneously. Each identity may have its own Relay account (or none at all).

When an identity is unlocked, the user can navigate to that identity's settings and sign into or create a Relay account. The app authenticates with the Relay and registers the current device's key for that identity (via proof of possession). The Relay session credentials (URL, session token or stored auth) are encrypted to the identity's public key and stored in the identity's configuration — consistent with how the workspace DB password is already stored encrypted to the identity key in `settings.json`. When the identity is locked, the Relay credentials are inaccessible.

This means:

- "Carsten @ 2pi" (work identity) might be connected to `relay.krillnotes.org`.
- "Carsten K" (personal identity) might be connected to a self-hosted Relay at `relay.kastner.family`, or might have no Relay at all.
- Both identities can be unlocked simultaneously, each maintaining its own independent Relay connection.

From this point, each workspace the identity participates in can optionally register a mailbox on that identity's Relay. The app manages mailbox registration automatically when a workspace is created, joined, or when a peer's channel is set to `relay`.

### 13.2 Connection Management

The app maintains a persistent WebSocket connection to each active identity's Relay when online. If the user has two identities unlocked, each connected to a different Relay, two WebSocket connections are active simultaneously. Inbound bundles are pushed in real-time per connection. If a WebSocket drops, the app falls back to periodic polling for that Relay. If a Relay is unreachable, the app continues operating normally — manual and folder-based channels are unaffected, and relay-channel bundles queue locally until the connection is restored. When an identity is locked, its Relay connection is closed.

### 13.3 Peer Channel UI

The workspace peer list displays each peer's channel configuration and sync status:

```
Workspace: Project Alpha — Peers
────────────────────────────────────────────────
Carsten (phone) ← you
  Channel: Relay (relay.krillnotes.org)
  Last sync: 10 minutes ago
  Status: ● Connected

Bob (ocean-maple-thunder)
  Channel: Relay (relay.krillnotes.org)
  Last sync: 2 minutes ago
  Status: ● Connected

Carol (river-stone-falcon)
  Channel: Manual (email)
  Last sync: 3 days ago
  Status: ○ 2 bundles in outbox

Dave (forest-peak-seven)
  Channel: Folder (/shared/syncthing/dave)
  Last sync: 6 hours ago
  Status: ● Watching
```

The user's own devices appear in the peer list alongside other people's devices. The UI distinguishes them with a "← you" marker. Each peer entry — whether another person or the user's own device — has its own channel configuration.

---

## 14. Trust and Security Properties

### 14.1 What the Relay Can See

The Relay has access to the unencrypted .swarm header, which reveals: workspace IDs, sender and recipient device keys, bundle mode (invite/accept/snapshot/delta), bundle size, and timing of exchanges. This is transport-level metadata — equivalent to what an email server sees (sender, recipient, timestamp, size) without seeing the email body.

### 14.2 What the Relay Cannot See

The Relay cannot see any workspace content: note titles, note bodies, field values, tags, tree structure, attachments, schema definitions, scripts, RBAC permissions, or any other data within the encrypted .swarm payload. If the Relay is compromised, the attacker gets encrypted blobs and routing metadata. No workspace content is exposed.

### 14.3 Delivery Integrity

The combination of proof-of-possession during registration (Section 5.5) and key-based routing (Section 8.2) provides a strong delivery guarantee: bundles are only delivered to accounts whose holder has proven they own the private key corresponding to a device key listed in the bundle's recipient list. This means:

- An attacker cannot register someone else's device key to intercept their bundles (proof-of-possession prevents this).
- An attacker cannot receive bundles for a workspace they're not invited to (key-based routing means bundles are only delivered to device keys the sender chose to encrypt for).
- Even if an attacker registers a mailbox for a workspace, they receive nothing unless the sender explicitly includes their device key as a recipient — which only happens if they've been legitimately invited and their key is in the sender's peer registry.

The Relay achieves this without ever decrypting payloads or understanding workspace membership. The security property emerges from the combination of verified key ownership and header-based recipient matching.

### 14.4 Metadata Exposure

Users should be aware that using the Relay exposes sync metadata to the Relay operator. For users who consider this unacceptable, manual exchange and shared-folder channels remain fully available. The per-peer-per-workspace channel model means a user can use the Relay for low-sensitivity workspaces while keeping sensitive workspaces on manual channels — the choice is granular.

### 14.5 Self-Hosting

The Relay is open source (MPL 2.0) and can be self-hosted. An organisation, team, or family can run their own Relay instance and point their KrillNotes apps at it. This eliminates third-party metadata exposure entirely. The default `relay.krillnotes.org` (hosted by 2pi Software) is the zero-config option for convenience.

---

## 15. API Surface

The Relay exposes a headless REST API consumed by the KrillNotes desktop app. There is no web dashboard or admin UI in the initial version — the `admin` role (Section 5.4) is reserved for a future administration interface.

### 15.1 Authentication

| Endpoint | Method | Description |
|---|---|---|
| `POST /auth/register` | — | Begin registration (email, password, first device public key). Returns encrypted challenge nonce. |
| `POST /auth/register/verify` | — | Complete registration by returning decrypted nonce (proof of possession). Activates account with first device key. |
| `POST /auth/login` | — | Authenticate, receive session token |
| `POST /auth/logout` | Auth | Invalidate session |
| `POST /auth/reset-password` | — | Request password reset email |
| `POST /auth/reset-password/confirm` | — | Complete password reset with token |

### 15.2 Account & Device Management

| Endpoint | Method | Description |
|---|---|---|
| `GET /account` | Auth | Retrieve account details (email, identity UUID, device keys, role, storage used) |
| `DELETE /account` | Auth | Flag account for deletion |
| `POST /account/devices` | Auth | Register a new device key (begins proof-of-possession challenge) |
| `POST /account/devices/verify` | Auth | Complete proof-of-possession for a new device key |
| `DELETE /account/devices/{device_key}` | Auth | Remove a device key from the account |

### 15.3 Workspace Mailboxes

| Endpoint | Method | Description |
|---|---|---|
| `POST /mailboxes` | Auth | Register a mailbox for a workspace |
| `DELETE /mailboxes/{workspace_id}` | Auth | Remove a mailbox |
| `GET /mailboxes` | Auth | List all registered mailboxes with pending bundle counts |

### 15.4 Bundle Transfer

| Endpoint | Method | Description |
|---|---|---|
| `POST /bundles` | Auth | Upload a .swarm bundle (routed by header) |
| `GET /bundles` | Auth | List pending bundles across all mailboxes |
| `GET /bundles/{bundle_id}` | Auth | Download a specific bundle |
| `DELETE /bundles/{bundle_id}` | Auth | Acknowledge receipt (Relay deletes the bundle) |

### 15.5 Invitation Relay

| Endpoint | Method | Description |
|---|---|---|
| `POST /invites` | Auth | Register an invite mailbox (pairing token + expiry) |
| `GET /invites/{pairing_token}` | Auth | Retrieve invite payload (for link-based flow) |
| `POST /invites/{pairing_token}/accept` | Auth | Submit an accept .swarm for this invite |
| `DELETE /invites/{pairing_token}` | Auth | Revoke an invite |

### 15.6 .cloud Broadcast

| Endpoint | Method | Description |
|---|---|---|
| `POST /broadcasts` | Auth | Upload a .cloud bundle for public hosting |
| `GET /broadcasts` | Auth | List the account's published broadcasts |
| `GET /broadcasts/{broadcast_id}` | Public | Download a .cloud bundle (no authentication required) |
| `DELETE /broadcasts/{broadcast_id}` | Auth | Remove a published broadcast |

### 15.7 Real-Time

| Endpoint | Method | Description |
|---|---|---|
| `WS /stream` | Auth | WebSocket for real-time bundle push notifications |

---

## 16. Deployment Model

### 16.1 Self-Hostable

The Relay ships as a single Docker image with a SQLite backend (suitable for small to medium deployments) or configurable Postgres backend (for larger deployments). The target deployment complexity: `docker run -p 8080:8080 -v relay-data:/data krillnotes-relay`.

### 16.2 Hosted Default

2pi Software operates the default instance at `relay.krillnotes.org` as a free service for KrillNotes users, with storage and bandwidth limits governed by the Relay configuration file (see Section 18). This is the zero-config option: users sign up, link their app, and start syncing.

### 16.3 Technology

The Relay is a Rust binary (consistent with the KrillNotes core stack). It has no dependency on krillnotes-core — it is a standalone service that speaks HTTP/WebSocket and stores opaque blobs. It could theoretically be implemented in any language, but Rust keeps the operational profile small and the deployment simple.

### 16.4 Licensing

MPL 2.0, consistent with KrillNotes.

---

## 17. Deliberate Limitations (Upgrade Path to OPswarm Base)

The following limitations are intentional. They keep the Relay simple and create clear differentiation for the commercial OPswarm Base product. The key distinction is that the Relay is a **transport pipe** — it moves opaque files between peers. OPswarm Base is a **protocol peer** — it actively participates in workspace management, decrypts content, applies RBAC, and runs automated pipelines.

| Limitation | Why It Matters | OPswarm Base Alternative |
|---|---|---|
| One identity per account | Teams can't use institutional identities | Base supports institutional HSM-backed identities |
| No protocol participation | Relay never decrypts, merges, or applies RBAC — it's a transport, not a peer | Base is a full peer: decrypts, merges, enforces RBAC, resolves conflicts |
| No operational monitoring | Nobody can see sync freshness or detect stale peers | Base provides monitoring dashboard (clock drift, rejected ops, channel health) |
| No automated .cloud generation | Users must manually generate and publish .cloud bundles (Section 12) | Base automatically generates .cloud deltas after each merge cycle from public subtrees |
| No integration layer | No VERSD, knowledge graph, or GIS connectors | Base provides full integration API (inbound and outbound) |
| Storage/bandwidth caps | Practical limit on high-volume workspaces | Base sized for enterprise throughput |
| No RBAC-aware routing | Relay can't filter content by permission (it can't read it) | Base applies RBAC to outbound bundles, enabling subtree-scoped snapshots |
| No compliance anchoring | No hash-chained audit archive or evidence packages | Base provides canonical compliance record with organisational signature |

---

## 18. Rate Limiting and Quotas

All rate limits, storage quotas, and retention periods are configurable via the Relay's configuration file. Each Relay operator chooses their own limits appropriate to their deployment context. An organisation hosting a restricted Relay for their teams can offer much higher limits than the public hosted instance.

Configurable parameters include: maximum bundle size (bytes), maximum storage per account (bytes), maximum bundles per account per hour, bundle retention period (days), and invite mailbox maximum lifetime (days).

The default configuration for the 2pi-hosted instance at `relay.krillnotes.org` will be set conservatively for a free public service. Self-hosted instances can raise or remove limits as needed.

---

## 19. Bundle Deduplication

The Relay does not perform bundle deduplication — it cannot inspect encrypted payloads to determine whether two bundles contain overlapping operations. Deduplication is a **client-side responsibility**.

If a user receives the same operations via multiple channels (e.g., both the Relay and a shared folder due to misconfiguration), the KrillNotes app handles this gracefully through the existing operation log. Operations are applied idempotently — applying the same signed operation twice has no effect. When the app detects that a bundle from the Relay contains only operations that have already been applied (e.g., from a folder-based exchange), it can acknowledge and delete the Relay copy without re-applying anything.

---

## 20. Open Questions

- **Notification preferences:** Should the Relay support push notifications (mobile, email digest) when bundles arrive and the user's app is offline? This adds complexity but improves the experience for infrequent users. Deferred to a later phase.

- **Admin UI:** A web-based admin interface for account management, storage monitoring, and configuration would be useful for Relay operators. The user role model (`user`/`admin`) is in place from day one. The admin UI itself is deferred to a later phase.

- **Channel update protocol:** The mechanism for propagating channel preference changes between peers needs formal specification. A signed channel-update message is the likely approach, but the exact payload format and distribution semantics should be defined.

---

## 21. Implementation Phases

| Phase | Delivers | Dependencies |
|---|---|---|
| **R1** | Core Relay service: accounts (with roles), public key verification (proof of possession), mailboxes, key-based bundle routing, REST API, password reset | Core Protocol Phase 1 (bundle generation) |
| **R2** | WebSocket real-time push, desktop app Relay transport adapter, multi-device session management | R1 |
| **R3** | Invite relay endpoint, link-based invitation flow, .cloud broadcast hosting | R1 + Core Protocol Phase 2 (identity, invite/accept) |
| **R4** | Self-hosting Docker image, configurable rate limits, documentation, hosted instance at relay.krillnotes.org | R1–R3 |
| **R5** | Admin web UI, push notifications (mobile/email digest) | R4 |

R1–R2 can run in parallel with Core Protocol Phases 1–2. R3 depends on the invitation handshake implementation being complete; .cloud hosting reuses the same public-endpoint infrastructure as the invite link flow. R5 is a convenience phase — the Relay is fully functional after R4.

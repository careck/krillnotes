# SWARM PROTOCOL — Access Control

**Version 1.0 — March 2026**

*Companion to Swarm Protocol Unified Design v0.7*

This document is the authoritative specification for access control lists (ACLs), permission enforcement, and revocation in the Swarm protocol. It consolidates and supersedes §15–18 of the Unified Design Specification v0.7, incorporates the Design Addendum amendments, and replaces the earlier RBAC model with a fine-grained ACL system.

The Swarm sync design (§1–14, §19–27) references this document for all permission-related behaviour.

**Related specification:** Trust levels, identity verification, and contact verification are specified separately in the *Swarm Trust & Verification Specification* (`Swarm_Trust_Verification_Spec`). Trust and verification are informational systems that have no interaction with ACLs — they do not gate or influence permission grants.

### ACL Architecture Overview

Krill Notes uses three layers of ACL, each governing a different resource type:

| Layer | Resource | Permission Bits | Described in |
|---|---|---|---|
| **Note-level** | Individual notes in the tree | `p` `d` `w` `o` `c` `r` (6 bits) | §1 |
| **Group-level** | Group membership management | `m` `r` `a` (3 bits) | §2.3 |
| **Workspace-level** | Structural workspace operations | `a` `s` `e` `g` `x` `n` (6 bits) | §3 |

All three layers share the same ACL mechanics: principals (users or groups), allow/deny entries, and signed `granted_by` chains. Note-level ACLs additionally support tree inheritance via the `inherit` flag.

### Bitmask Encoding

All permission sets are stored and transported as **integer bitmasks**. Higher bit positions represent more powerful permissions. This enables efficient bitwise operations for permission checks and ensures that higher numeric values consistently represent greater access.

**Note-level permissions (6 bits, 0–63):**

| Bit | Position | Value | Permission | Description |
|---|---|---|---|---|
| `p` | 5 | 32 | **Permissions** | Manage ACL entries on this note |
| `d` | 4 | 16 | **Delete** | Delete **any** note |
| `w` | 3 | 8 | **Write** | Edit **any** note |
| `o` | 2 | 4 | **Own** | Edit or delete only notes **you authored** |
| `c` | 1 | 2 | **Create** | Create child notes |
| `r` | 0 | 1 | **Read** | View content, fields, tags, attachments |

Common profiles:

| Profile | Bits | Integer | Description |
|---|---|---|---|
| Read only | `r` | **1** | Observer |
| Read + create | `rc` | **3** | Can add notes but not edit existing |
| Contributor | `roc` | **7** | Create, edit/delete own |
| Editor | `rwoc` | **15** | Edit any note, create, own |
| Full data | `rwdoc` | **31** | All data ops, no permission management |
| Full admin | `pdwocr` | **63** | Full control including ACL management |

**Group-level permissions (3 bits, 0–7):**

| Bit | Position | Value | Permission | Description |
|---|---|---|---|---|
| `m` | 2 | 4 | **Manage** | Rename, delete group, modify group ACL |
| `r` | 1 | 2 | **Remove** | Remove users from group |
| `a` | 0 | 1 | **Add** | Add users to group |

Common profiles: add only = **1**, add + remove = **3**, full management = **7**.

**Workspace-level permissions (6 bits, 0–63):**

| Bit | Position | Value | Permission | Description |
|---|---|---|---|---|
| `a` | 5 | 32 | **Admin** | Modify workspace ACL entries |
| `s` | 4 | 16 | **Script govern** | Rhai script lifecycle (global effect) |
| `e` | 3 | 8 | **Everyone govern** | Manage `everyone` group note ACL entries |
| `g` | 2 | 4 | **Group create** | Create new groups |
| `x` | 1 | 2 | **Export** | Export workspace |
| `n` | 0 | 1 | **New root note** | Create root-level notes |

Common profiles: root notes only = **1**, export + root notes = **3**, full workspace admin = **63**.

**Bitwise operations:**

```
# Check: does user have read + create?
required = READ | CREATE          # 1 | 2 = 3
has_access = (user_perms & required) == required

# Check: is granted a subset of granter's permissions? (permission cap)
valid_grant = (granted & granter_perms) == granted

# Union of multiple group permissions
combined = group_a_perms | group_b_perms
```

---

## 1. Note-Level Access Control

Krill Notes implements fine-grained access control at the note level using Access Control Lists (ACLs) with permission inheritance through the tree hierarchy. Each note can have an ordered list of ACL entries that define who can do what. This serves two purposes: it controls access precisely, and it reduces sync conflicts by limiting writers on any given subtree.

### 1.1 Permission Bits

Six atomic permissions govern all operations on a note. See the Bitmask Encoding section above for bit positions and integer values.

**Authorship** is determined by the identity that signed the `CreateNote` operation. This is immutable and cryptographically verifiable.

**Relationship between `w`, `o`, and `d`:**

- `w` (8) subsumes the write aspect of `o` (4) — if you have `w`, you can edit any note regardless of authorship.
- `d` (16) subsumes the delete aspect of `o` (4) — if you have `d`, you can delete any note regardless of authorship.
- `o` (4) alone grants both write and delete, but only on notes the user authored.
- A user with permissions **7** (`roc`) can read everything, create new notes, and edit or delete their own notes — but cannot touch notes authored by others.
- `p` (32) is independent of all other bits — a user with **33** (`r` + `p`) can read and manage who has access, but cannot edit or create notes themselves. Conversely, a user with **31** (`rdwoc`) has full data control but cannot modify ACL entries.

**The `p` bit and the permission-capped rule:** Having `p` authorises a user to set ACL entries on a note, but the entries they create are still capped by their own effective permissions. A user with **39** (`p` + `roc` = 32 + 7) can grant at most **7** to others (they cannot grant `w`, `d`, or `p` because they don't hold those bits). To grant `p` to someone else, you must hold `p` yourself.

**Moving a note** is decomposed into two atomic operations: delete from the source parent and create at the destination parent. The user needs either `d` (16) or `o` (4, if they authored the note being moved) on the source parent, plus `c` (2) on the destination parent.

### 1.2 ACL Entries

Each note may have zero or more ACL entries. An entry (also called an Access Control Entry, or ACE) consists of:

| Field | Type | Description |
|---|---|---|
| `note_id` | ID | The note this entry applies to |
| `principal_id` | ID | A user identity (Ed25519 public key) or a group (UUID) |
| `principal_type` | `user` \| `group` | Whether the principal is an individual user or a group |
| `entry_type` | `allow` \| `deny` | Whether this entry grants or blocks access |
| `permissions` | Integer | Bitmask (0–63) encoding a subset of `p`, `d`, `w`, `o`, `c`, `r` |
| `inherit` | Boolean | If `true`, the entry applies to this note and all descendants. If `false`, it applies to this note only. Default: `true` |
| `granted_by` | ID | The identity that signed this ACL entry (for delegation chains and revocation) |

Example ACL for a note:

```
Note: "Situation Reports"
  field-ops    allow  7   (roc)     inherit:true     granted_by:alice
  command      allow  63  (pdwocr)  inherit:true     granted_by:alice
  analysts     allow  1   (r)       inherit:true     granted_by:alice
  everyone     allow  1   (r)       inherit:true     granted_by:alice
```

In this example: field-ops members can read everything, create new sitreps, and edit or delete their own — but cannot modify reports authored by others or change permissions. Command has full control including permission management. Analysts and everyone else can read only.

### 1.3 Key Constraints

- **Default-deny:** If no ACL entry (direct or inherited) grants a permission, access is denied.
- **Deny overrides allow:** At the same tree level, a `deny` entry takes precedence over an `allow` entry for the same principal.
- **User entries override group entries:** A direct user ACE takes precedence over a group ACE at the same tree level.

---

## 2. Groups

Groups provide an indirection layer between users and ACL entries, making it practical to manage permissions for many users at once. Groups are also ACL-controlled resources themselves, enabling delegated group management.

### 2.1 Group Model

- Groups are **flat** (no nesting — a group cannot contain other groups as members).
- Groups are **workspace-wide** — a group exists once and can appear in ACL entries on any note.
- A user can belong to **multiple groups**.
- A note can have ACL entries for **multiple groups**.

### 2.2 The `everyone` Group

Every workspace has a built-in group called `everyone`. All users with access to the workspace are implicit members and cannot be removed from it.

**Special rules for `everyone`:**

- Membership is automatic and cannot be modified (no AddGroupMember or RemoveGroupMember operations apply).
- The `everyone` group has no group-level ACL — it cannot be renamed, deleted, or have its membership managed.
- Note-level ACL entries referencing `everyone` can only be created, modified, or removed by principals who hold the `e` (8) permission on the workspace ACL (§3).
- The `everyone` group's note-level ACL entries can vary across the tree (e.g., `everyone allow 1` at the root, `everyone deny 63` on a private subtree).

### 2.3 Group-Level ACL

Each group (except `everyone`) has its own ACL that controls who can manage the group. See the Bitmask Encoding section for bit positions and integer values of `m` (4), `r` (2), `a` (1).

Group-level ACL principals can be **users or other groups**. This enables patterns like "the admin group can manage all other groups."

Example group ACL:

```
Group: "field-ops"
ACL:
  alice      allow  7  (mra)   full group management
  bob        allow  1  (a)     can add members only
  admin      allow  3  (ra)    admin group: can add and remove members
```

**Group-level ACL resolution** follows the same precedence rules as note-level ACLs:

- Deny overrides allow for the same principal.
- User entries override group entries.
- Multiple group memberships: deny on any group wins; if all allow, permissions are unioned.

### 2.4 Group Operations

| Operation | Required Permission | Effect |
|---|---|---|
| `CreateGroup` | Workspace `g` (4) | New named group created; creator receives initial **7** (mra) on the group |
| `DeleteGroup` | `m` (4) on the group | Group dissolved; note ACL entries referencing it become inert |
| `RenameGroup` | `m` (4) on the group | Group name updated |
| `AddGroupMember` | `a` (1) on the group | Identity joins group |
| `RemoveGroupMember` | `r` (2) on the group | Identity leaves group |
| `SetGroupPermission` | `m` (4) on the group | Create/modify a group-level ACL entry |
| `RevokeGroupPermission` | `m` (4) on the group, or entry granter | Remove a group-level ACL entry |

Group operations are signed and logged like all other operations, making group membership changes auditable and verifiable by every peer.

### 2.5 Group Storage

```sql
groups (
    id   TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
)

group_members (
    group_id TEXT NOT NULL REFERENCES groups(id),
    user_id  TEXT NOT NULL,
    PRIMARY KEY (group_id, user_id)
)

group_acl (
    group_id       TEXT NOT NULL REFERENCES groups(id),
    principal_id   TEXT NOT NULL,
    principal_type TEXT NOT NULL CHECK(principal_type IN ('user','group')),
    entry_type     TEXT NOT NULL CHECK(entry_type IN ('allow','deny')),
    permissions    INTEGER NOT NULL,
    granted_by     TEXT NOT NULL,
    PRIMARY KEY (group_id, principal_id, principal_type)
)
```

---

## 3. Workspace-Level Access Control

The workspace itself has an ACL that controls structural operations — capabilities that affect the workspace as a whole rather than individual notes or groups.

### 3.1 Workspace Permission Bits

See the Bitmask Encoding section for bit positions and integer values. The bits are ordered by impact: `a` (32) > `s` (16) > `e` (8) > `g` (4) > `x` (2) > `n` (1).

### 3.2 Workspace ACL Entries

The workspace ACL uses the same principal/allow/deny/granted_by model as note and group ACLs:

| Field | Type | Description |
|---|---|---|
| `principal_id` | ID | A user identity (Ed25519 public key) or a group (UUID) |
| `principal_type` | `user` \| `group` | Whether the principal is an individual user or a group |
| `entry_type` | `allow` \| `deny` | Whether this entry grants or blocks the capability |
| `permissions` | Integer | Bitmask (0–63) encoding a subset of `a`, `s`, `e`, `g`, `x`, `n` |
| `granted_by` | ID | The identity that signed this entry |

Example workspace ACL:

```
Workspace ACL:
  alice      allow  63  (asegxn)  root owner: full workspace admin
  admin      allow  29  (segxn)   admin group: everything except workspace ACL admin
  bob        allow  16  (s)       bob: can manage scripts
  everyone   deny   63  (asegxn)  default: no workspace-level powers
```

### 3.3 Resolution

Workspace ACL resolution follows the same precedence rules as the other ACL layers:

- Deny overrides allow for the same principal.
- User entries override group entries.
- Multiple group memberships: deny on any group wins; if all allow, permissions are unioned.
- Default-deny: if no entry matches, the capability is denied.

### 3.4 Bootstrap

When a workspace is created, the root owner identity is granted **63** (all workspace permissions) as the initial workspace ACL entry. This is the only hardcoded permission in the system — from this point forward, all authority flows through ACL delegation.

The root owner can then:

1. Create an `admin` group (using `g`).
2. Grant the `admin` group workspace permissions (using `a`).
3. Add trusted users to the `admin` group.
4. Those admins can now manage the workspace even if the root owner is unavailable.

### 3.5 Workspace ACL Storage

```sql
workspace_acl (
    principal_id   TEXT NOT NULL,
    principal_type TEXT NOT NULL CHECK(principal_type IN ('user','group')),
    entry_type     TEXT NOT NULL CHECK(entry_type IN ('allow','deny')),
    permissions    INTEGER NOT NULL,
    granted_by     TEXT NOT NULL,
    PRIMARY KEY (principal_id, principal_type)
)
```

---

## 4. Permission Inheritance & Resolution

### 4.1 Note-Level Tree-Walk Resolution

Permissions are set on individual notes via ACL entries and inherited by descendants (when `inherit` is `true`). When checking whether user X has permission P on note N, the system resolves as follows:

1. **Collect applicable entries.** Walk up the tree from note N to the workspace root. At each node, collect ACL entries that apply to user X — either directly (by user ID) or via group membership. Skip entries where `inherit = false` unless the entry is on the exact note being checked.

2. **Evaluate from most specific to least specific.** Entries on the note itself take precedence over entries on the parent, which take precedence over entries on the grandparent, and so on up to the root.

3. **At each tree level, apply precedence rules:**
   - **Deny before allow:** A `deny` entry overrides an `allow` entry for the same principal at the same level.
   - **User before group:** A user-specific entry overrides a group entry at the same level.
   - **Multiple groups:** If the user is in multiple groups with entries at the same level, `deny` on any group wins. If all are `allow`, the permissions are **unioned** (bitwise OR of the bitmasks).

4. **First match wins.** Once a definitive answer (allow or deny) is found for permission P, the walk stops. If the walk reaches the root with no applicable entry, access is **denied** (default-deny).

### 4.2 Own-Permission (`o`) Resolution

When the system checks whether user X can write to or delete note N via the `o` bit:

1. Resolve user X's effective permissions on note N using the standard tree-walk (§4.1).
2. If the resolved permissions include `w` (8, for writes) or `d` (16, for deletes), access is granted regardless of authorship.
3. If the resolved permissions include `o` (4) but not `w`/`d`, check whether user X is the author of note N (the identity that signed the `CreateNote` operation).
4. If X is the author, grant the write or delete. If not, deny.

This means `o` is evaluated at the point of operation, not during the tree-walk itself. The tree-walk resolves what bits the user holds; the `o` bit is then checked against authorship at enforcement time.

### 4.3 Resolution Examples

```
Workspace Root
  ACL: everyone  allow  1   (r)       inherit:true
├─ Situation Reports
│  ACL: field-ops  allow  7   (roc)     inherit:true
│       command    allow  63  (pdwocr)  inherit:true
│  ├─ Sitrep 001 (author: bob, field-ops member)
│  ├─ Sitrep 002 (author: carol, field-ops member)
│  └─ Sitrep 003 (author: dave, command member)
├─ Analysis
│  ACL: analysts   allow  7   (roc)     inherit:true
│       field-ops  deny   63  (pdwocr)  inherit:true
│  ├─ Intel Brief (author: eve, analyst)
│  └─ Threat Assessment (author: eve, analyst)
└─ Command Only
   ACL: everyone  deny   63  (pdwocr)  inherit:true
        command   allow  63  (pdwocr)  inherit:true
```

- **Bob on "Sitrep 001":** Has **7** (roc) via field-ops. Bob authored this note, so `o` grants him write + delete. He can edit and delete it.
- **Bob on "Sitrep 002":** Has **7** (roc) via field-ops. Carol authored this note. `o` does not apply — Bob can read but not edit or delete.
- **Dave on "Sitrep 001":** Has **63** (pdwocr) via command. `w` and `d` override authorship checks — Dave can edit and delete any note. `p` lets him manage permissions.
- **Field-ops member on "Analysis":** Denied — the deny **63** entry for field-ops blocks all access, overriding the inherited **1** from the root.
- **Eve on "Intel Brief":** Has **7** (roc) via analysts. Eve authored this note, so `o` grants write + delete.
- **Anyone on "Command Only":** Denied via `everyone deny` — unless they are in the command group.

### 4.4 Permission Storage

```sql
note_acl (
    note_id        TEXT NOT NULL REFERENCES notes(id),
    principal_id   TEXT NOT NULL,
    principal_type TEXT NOT NULL CHECK(principal_type IN ('user','group')),
    entry_type     TEXT NOT NULL CHECK(entry_type IN ('allow','deny')),
    permissions    INTEGER NOT NULL,
    inherit        INTEGER NOT NULL DEFAULT 1,
    granted_by     TEXT NOT NULL,
    PRIMARY KEY (note_id, principal_id, principal_type)
)
```

### 4.5 Tree Move Interaction

Moving a note between subtrees requires delete capability on the source parent (`d` = 16, or `o` = 4 if the user authored the note) and `c` (2) on the destination parent. If the user lacks either, the move is rejected.

When a note is moved to a new subtree, it inherits the ACL context of its new parent. ACL entries set directly on the moved note itself are preserved. This is an immediate consequence of the tree-walk inheritance model.

---

## 5. Delegation & Invitation

### 5.1 Invitation Model

Invitation to a workspace is tied to group membership management. The ability to invite a new user depends on group-level ACL permissions, not note-level permissions.

**Invitation rule:** A user can invite another person to the workspace if and only if they hold `a` (1) permission on at least one group.

When inviting, the inviter can offer the invitee membership in any combination of groups where the inviter holds `a`. The invitee's note-level access is then determined entirely by the note-level ACL entries assigned to those groups.

**Example invitation flow:**

```
Setup:
  Alice (root owner) creates groups: field-ops, analysts, command
  Alice grants Bob 'a' (1) on field-ops via group ACL
  Alice sets note ACL: field-ops gets 7 (roc) on /Situation Reports

Invitation:
  Bob invites Carol
  Bob can offer: field-ops membership (only group where he has 'a')
  Carol accepts → Carol becomes a field-ops member
  Carol's note access: 7 (roc) on /Situation Reports (inherited from group)
```

The inviter does not grant note-level permissions directly. The inviter's role is to bring people into the workspace and assign them to appropriate groups. Note-level permissions are managed separately by whoever has authority to set ACL entries on notes.

### 5.2 Permission-Capped Note ACL Grants

For direct note-level ACL entries (granting a specific user or group access to a specific note), the permission-capped principle still applies:

**Principle:** You can grant note-level permissions up to but not exceeding your own effective permissions on that note.

A user with **7** (roc) on a note cannot grant **63** (pdwocr) — they lack `w`, `d`, and `p`. A user with **1** (r) can only grant **1**. A user must hold `p` (32) on a note to create or modify ACL entries on it at all.

**Formally:** `(granted & granter_perms) == granted` must be true, and `granter_perms & 32 != 0` (the `p` bit must be set).

**Security invariant:** The overall permission surface area is bounded by the root owner's original grants. Delegation cannot create permissions that don't already exist in the subtree.

### 5.3 Delegation Chains

Permission grants form verifiable chains. Each ACL entry (a `SetNoteAcl` operation) is signed by the granting identity, and any receiving peer can walk the chain backwards to verify that every link was valid at the time of signing:

```
Alice (root owner)         → "field-ops gets 7 (roc) on /Situation Reports"
Alice (root owner)         → "Bob gets 1 (a) on group field-ops"
Bob (field-ops, has 'a')   → "Carol is added to field-ops"
Carol (field-ops member)   → Carol now has 7 (roc) on /Situation Reports via group
```

Every link is valid: Alice had authority to set the note ACL and the group ACL. Bob had `a` on field-ops to add Carol. Carol's note access derives from her group membership. The chain is verifiable locally by any peer without contacting a central authority.

### 5.4 Chain Cascade on Revocation

If Alice revokes Bob's `a` permission on the field-ops group, Bob can no longer add members. However, Carol's existing membership in field-ops is **not automatically revoked** — she was validly added when Bob held `a`. To remove Carol, someone with `r` (2) permission on field-ops must explicitly remove her.

If Alice revokes the field-ops note ACL entry on /Situation Reports, all field-ops members (including Carol) lose access to that subtree. Any note-level ACL entries that were granted by field-ops members and depended on their field-ops-derived permissions are re-evaluated and may cascade to invalid (see §7.4).

### 5.5 Invitation Flow Integration

The invitation mechanics (described in the Swarm sync design, §12) are structurally unchanged. Both the known-contact path (1 exchange) and unknown-peer path (3 exchanges) work identically. The differences from the prior RBAC model are:

- The invitation payload carries group membership assignments rather than a named role or note-level permissions.
- The UI for the invitation form shows the groups the inviter can offer (those where they hold `a`), with checkboxes for group selection.
- Receiving peers verify the `AddGroupMember` operation against the group-level ACL to confirm the inviter held `a` at the time of signing.

---

## 6. Permission Enforcement

Every operation in Krill Notes is cryptographically signed by the author's private key. Verification is performed locally on every device when applying incoming operations from a .swarm or .cloud bundle. There is no server to enforce permissions — every device is its own enforcer.

### 6.1 Verification Pipeline

When an operation arrives in an inbound bundle, the receiving device:

1. *(For .swarm only)* Decrypts the payload using the recipient's private key and the per-recipient AES key wrapper.
2. Verifies the bundle-level signature against the sender's known public key. *(For .cloud, also verifies against the stored trusted fingerprint.)*
3. Verifies each operation's individual Ed25519 signature against the author's known public key.
4. Determines the operation's ACL layer and resolves permissions:
   - **Note operations:** Resolves effective permissions via note-level tree-walk (§4.1).
   - **Group operations:** Resolves group-level permissions (§2.3).
   - **Workspace operations:** Resolves workspace-level permissions (§3.3).
5. Checks that the resolved permissions include the required bit(s) for the operation type (see §6.2).
6. For operations requiring `o`: additionally verifies authorship of the target note (§4.2).
7. If all checks pass, the operation is applied. Otherwise, it is rejected and logged.

### 6.2 Operation-to-Permission Mapping

**Note operations:**

| Operation Type | Required Permission |
|---|---|
| Read note content | `r` (1) on the note |
| UpdateNote (title, properties) | `w` (8) on the note, or `o` (4) if author |
| UpdateField | `w` (8) on the note, or `o` (4) if author |
| SetTags | `w` (8) on the note, or `o` (4) if author |
| AddAttachment / RemoveAttachment | `w` (8) on the note, or `o` (4) if author |
| CreateNote | `c` (2) on the parent note |
| DeleteNote | `d` (16) on the note, or `o` (4) if author |
| MoveNote | (`d` (16) or `o` (4) if author) on source parent + `c` (2) on destination parent |
| SetPermission (note ACL entry) | `p` (32) on the note, and granter's perms ⊇ granted perms (§5.2) |
| SetPermission (everyone ACL entry) | Workspace `e` (8) |
| RevokePermission (note ACL entry) | `p` (32) on the note, or see §7.3 for additional revocation rights |

**Group operations:**

| Operation Type | Required Permission |
|---|---|
| CreateGroup | Workspace `g` (4) |
| DeleteGroup | `m` (4) on the group |
| RenameGroup | `m` (4) on the group |
| AddGroupMember | `a` (1) on the group |
| RemoveGroupMember | `r` (2) on the group |
| SetGroupPermission | `m` (4) on the group |
| RevokeGroupPermission | `m` (4) on the group, or entry granter |

**Workspace operations:**

| Operation Type | Required Permission |
|---|---|
| SetWorkspacePermission | Workspace `a` (32) |
| RevokeWorkspacePermission | Workspace `a` (32), or entry granter |
| CreateUserScript / UpdateUserScript / DeleteUserScript | Workspace `s` (16) |
| ExportWorkspace | Workspace `x` (2) |
| CreateRootNote | Workspace `n` (1) |

### 6.3 Modified Client Threat Model

The security assessment analysed four sub-threats:

| Threat | Risk | Mitigation |
|---|---|---|
| **A:** Modified client ignores ACLs locally | Contained to their device | Cannot be prevented; physical access reality |
| **B:** Modified client generates unauthorised operations | Primary threat | Every receiving peer validates signature + ACL on ingest; unauthorised ops rejected |
| **C:** Two colluding modified clients | Contained between them | Cannot infect honest peers; honest peer perimeter is the security boundary |
| **D:** Authorised liar (valid access, false data) | Human process problem | Non-repudiation: every entry permanently signed with author's identity key |

---

## 7. Revocation & Edge Cases

### 7.1 Revocation Is Eventually Consistent

In a decentralised system, revocation cannot be instantaneous. When Alice removes Bob's access, the RevokePermission operation must propagate to all peers via .swarm bundles. Until a peer receives the revocation, it may accept operations from Bob that were generated after the revocation.

> **Security Finding SA-002 (High): Revocation Propagation Gap**
>
> During the propagation window (hours over LoRa/sneakernet), peers continue accepting operations from revoked users. The original design specified retroactive rollback, which creates accountability problems — data that informed decisions disappears.
>
> **Resolution:** Implement a quarantine model for the commercial product. Operations from revoked users that post-date the revocation are flagged as "contested" rather than removed. Three operation states: valid, rejected, contested. See the Swarm sync design (§21) for the full data preservation specification.

### 7.2 Transport Encryption and Revocation

Transport encryption provides defence in depth. When a user's access is revoked:

1. The RevokePermission operation propagates to all peers via bundles.
2. Future .swarm bundles omit the revoked user from the recipients list.
3. The revoked user cannot decrypt any new .swarm bundles, even if they intercept them from a shared folder.
4. The revoked user's local database remains accessible (their SQLCipher password still works), but they are cryptographically cut off from all future updates.

Permission enforcement rejects unauthorised operations at the application level; transport encryption prevents unauthorised access at the file level.

### 7.3 Revocation Rights

**Note-level ACL revocation:**

- **Workspace admin:** Any principal with workspace `a` (32) can revoke any note ACL entry. This is the top-level administrative override.
- **Entry granter:** The identity that created an ACL entry (the `granted_by` field) can always revoke it.
- **Permission holder:** Any user with `p` (32) on a note can revoke ACL entries on that note, regardless of who issued them. This provides subtree-level administrative capability.
- **Others:** Cannot revoke entries they did not create and do not have `p` on.

**Group-level ACL revocation:**

- **Workspace admin:** Any principal with workspace `a` (32) can revoke any group ACL entry.
- **Entry granter:** Can revoke group ACL entries they created.
- **Group manager:** Any principal with `m` (4) on the group can revoke group ACL entries on that group.

**Workspace-level ACL revocation:**

- **Workspace admin:** Any principal with workspace `a` (32) can revoke workspace ACL entries.
- **Entry granter:** Can revoke workspace ACL entries they created.

### 7.4 Chain Cascade Mechanics

When a revocation breaks a delegation chain (§5.4), the cascade is evaluated as follows:

1. The RevokePermission operation is applied, removing the target ACL entry.
2. All SetPermission operations where `granted_by` matches the revoked principal are identified.
3. Each downstream grant is evaluated: does the granter still hold sufficient permissions to have issued it? If not, the grant is invalidated.
4. This evaluation recurses: invalidating Carol's grant triggers re-evaluation of grants Carol issued.
5. Invalidated grants are not deleted from the operation log (they are preserved for audit). Their effect on the working permission state is removed.

### 7.5 Group Membership Revocation

Removing a user from a group has an immediate effect on their effective permissions:

- All note ACL entries that granted the user access via that group no longer apply.
- Delegation chains where the user's authority derived from group membership are re-evaluated. If the user issued grants while a member, and those grants exceed what the user can now authorise (without the group membership), the downstream grants cascade to invalid.
- Group-level permissions the user held via the removed group are also re-evaluated. For example, if Bob was in the `admin` group which had `a` (1) on `field-ops`, and Bob is removed from `admin`, Bob loses the ability to add members to `field-ops`.
- Workspace-level permissions derived from the removed group are re-evaluated similarly.

### 7.6 Delete vs. Edit Conflict

If device A deletes a note while device B edits it, the delete wins by default. Edits to a deleted note are discarded during bundle application. The deleted note's data is retained in the operation log for audit and potential restoration.

The delete-vs-edit behaviour is configurable per schema (see Swarm sync design, §21.2). An AIIMS Situation Report schema can declare `on_delete_conflict: preserve` while a personal recipe schema keeps `delete_wins`.

### 7.7 Tree Move Conflicts

If two devices move the same note to different parents, the system applies LWW as the working state but flags the conflict for user review. Cycle detection is performed before applying any tree move — a move that would create a cycle is rejected.

---

## 8. ACL Operation Types

Every ACL change is recorded as a signed operation in the append-only operation log and transported to peers via delta bundles (.swarm files). Each operation follows the established pattern: `operation_id` (UUID), `timestamp` (HLC), `device_id`, an author field (`*_by` — the signer's Ed25519 public key), and `signature` (Ed25519 over canonical JSON).

Receiving peers verify each operation's signature, resolve the signer's permissions at the time of signing, and apply or reject accordingly. All identifiers (user IDs, group IDs, note IDs) are globally unique: user IDs are Ed25519 public keys (base64), group and note IDs are UUIDs. The `principal_type` field disambiguates users from groups.

### 8.1 Note-Level ACL Operations

**SetNoteAcl** — Create or update a note ACL entry.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `note_id` | String | Target note UUID |
| `principal_id` | String | User public key or group UUID |
| `principal_type` | String | `"user"` or `"group"` |
| `entry_type` | String | `"allow"` or `"deny"` |
| `permissions` | Integer | Bitmask (0–63) |
| `inherit` | bool | Applies to descendants |
| `granted_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `p` (32) on the target note. `(permissions & signer_perms) == permissions` must be true. For entries referencing the `everyone` group, workspace `e` (8) is required instead of `p`.

**RevokeNoteAcl** — Remove a note ACL entry.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `note_id` | String | Target note UUID |
| `principal_id` | String | Principal whose entry is revoked |
| `principal_type` | String | `"user"` or `"group"` |
| `revoked_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `p` (32) on the target note, OR be the original `granted_by` of the entry, OR hold workspace `a` (32).

### 8.2 Group Management Operations

**CreateGroup** — Create a new group.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `group_id` | String | UUID for the new group |
| `group_name` | String | Display name |
| `created_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold workspace `g` (4). On apply, the creator automatically receives **7** (mra) on the new group.

**DeleteGroup** — Dissolve a group.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `group_id` | String | UUID of group being dissolved |
| `deleted_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `m` (4) on the group. On apply, all note ACL entries referencing this group become inert.

**RenameGroup** — Change a group's display name.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `group_id` | String | UUID of group being renamed |
| `new_name` | String | New display name |
| `modified_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `m` (4) on the group.

**AddGroupMember** — Add a user to a group.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `group_id` | String | UUID of target group |
| `user_id` | String | Ed25519 public key of user being added (base64) |
| `added_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `a` (1) on the group.

**RemoveGroupMember** — Remove a user from a group.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `group_id` | String | UUID of target group |
| `user_id` | String | Ed25519 public key of user being removed (base64) |
| `removed_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `r` (2) on the group.

### 8.3 Group-Level ACL Operations

**SetGroupAcl** — Create or update a group ACL entry (controls who can manage the group).

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `group_id` | String | UUID of target group |
| `principal_id` | String | User public key or group UUID receiving management permissions |
| `principal_type` | String | `"user"` or `"group"` |
| `entry_type` | String | `"allow"` or `"deny"` |
| `permissions` | Integer | Bitmask (0–7) |
| `granted_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `m` (4) on the target group.

**RevokeGroupAcl** — Remove a group ACL entry.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `group_id` | String | UUID of target group |
| `principal_id` | String | Principal whose entry is revoked |
| `principal_type` | String | `"user"` or `"group"` |
| `revoked_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold `m` (4) on the group, OR be the original `granted_by`, OR hold workspace `a` (32).

### 8.4 Workspace-Level ACL Operations

**SetWorkspaceAcl** — Create or update a workspace ACL entry.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `principal_id` | String | User public key or group UUID |
| `principal_type` | String | `"user"` or `"group"` |
| `entry_type` | String | `"allow"` or `"deny"` |
| `permissions` | Integer | Bitmask (0–63) |
| `granted_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold workspace `a` (32).

**RevokeWorkspaceAcl** — Remove a workspace ACL entry.

| Field | Type | Description |
|---|---|---|
| `operation_id` | String | UUID |
| `timestamp` | HlcTimestamp | Hybrid Logical Clock |
| `device_id` | String | Originating device |
| `principal_id` | String | Principal whose entry is revoked |
| `principal_type` | String | `"user"` or `"group"` |
| `revoked_by` | String | Signer's Ed25519 public key (base64) |
| `signature` | String | Ed25519 signature (base64) |

*Verification:* Signer must hold workspace `a` (32), OR be the original `granted_by`.

### 8.5 Operation Summary

| Operation | ACL Layer | Verification Requires |
|---|---|---|
| `SetNoteAcl` | Note | `p` (32) on note (or workspace `e` (8) for everyone) |
| `RevokeNoteAcl` | Note | `p` (32) on note, or `granted_by`, or workspace `a` (32) |
| `CreateGroup` | Group mgmt | Workspace `g` (4) |
| `DeleteGroup` | Group mgmt | `m` (4) on group |
| `RenameGroup` | Group mgmt | `m` (4) on group |
| `AddGroupMember` | Group mgmt | `a` (1) on group |
| `RemoveGroupMember` | Group mgmt | `r` (2) on group |
| `SetGroupAcl` | Group ACL | `m` (4) on group |
| `RevokeGroupAcl` | Group ACL | `m` (4) on group, or `granted_by`, or workspace `a` (32) |
| `SetWorkspaceAcl` | Workspace | Workspace `a` (32) |
| `RevokeWorkspaceAcl` | Workspace | Workspace `a` (32), or `granted_by` |

### 8.6 JoinWorkspace Integration

The existing `JoinWorkspace` operation is retained but its semantics change. When a peer joins a workspace via invitation, the inviter also issues one or more `AddGroupMember` operations to place the new peer in the appropriate groups. The `JoinWorkspace` operation itself establishes the peer's identity in the workspace; the `AddGroupMember` operations (signed by the inviter, who must hold `a` on the relevant groups) establish the peer's group memberships and, by extension, their note-level access.

---

## 9. The Root Owner

The root owner is the identity that created the workspace. They are the initial authority from which all other permissions flow, but their privileges are fully delegatable.

### 9.1 Initial Authority

When a workspace is created, the root owner receives:

- **63** on the workspace ACL (all workspace-level permissions).
- **63** on the workspace root note (all note-level permissions, inheriting to the entire tree).

From this starting point, the root owner can delegate any capability to other users or groups. There are no hardcoded "root-owner-only" checks in the system — all authority derives from ACL entries.

### 9.2 Delegation and Continuity

Because all privileges flow through ACL entries, the root owner can ensure workspace continuity by:

1. Creating an `admin` group (workspace `g` = 4).
2. Granting the `admin` group workspace **63** (workspace `a` = 32 permits this).
3. Adding trusted users to the admin group.
4. Those admins can now perform all workspace operations — including managing scripts (`s` = 16), exporting (`x` = 2), governing `everyone` permissions (`e` = 8), and granting further workspace permissions (`a` = 32) — even if the root owner's device is permanently unavailable.

### 9.3 Root Owner and the Swarm Server

> **Security Finding SA-004 (Critical → Mitigated): Root Owner Single Point of Failure**
>
> The root owner identity is bound to a single Ed25519 keypair. If the root owner's device is destroyed or the person rotates off shift, workspace operations could be frozen.
>
> **Resolution:** The workspace ACL delegation model (§3) allows the root owner to grant full workspace authority (**63**) to an admin group before any disruption occurs. For the commercial product, the Swarm Server holds the root owner keypair in an HSM with institutional control. For open-source Krill Notes, the recommended practice is to delegate workspace `a` (32) to at least two independent principals as a continuity measure.

---

*End of Access Control Specification*

*Swarm Protocol — ACL Specification v1.0*

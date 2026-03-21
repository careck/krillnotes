# KRILLNOTES — Role-Based Access Control

**Version 2.0 — March 2026**

*Supersedes RBAC Specification v1.0*

This document specifies the role-based access control model for Krillnotes. It defines roles, capabilities, permission inheritance, invitation rules, and revocation behavior.

This specification is purely behavioral — it describes *what* the permission model does, not *how* it integrates into the codebase. Implementation details (the `PermissionGate` trait, crate structure, operation enum changes, workspace integration) are defined in the **Permission Gate Specification v1.0**.

Trust levels, identity verification, and contact verification are separate concerns and are not part of the RBAC model.

---

## 1. Role Definitions

Four roles, strictly ordered. The ordering is used for role-capped delegation (§4).

**Role hierarchy:** Root Owner > Owner > Writer > Reader

| Role | Scope | Description |
|---|---|---|
| **Root Owner** | Workspace | The identity that created the workspace. Exactly one per workspace. Holds all capabilities at every level. |
| **Owner** | Subtree | Full control within an assigned subtree: read, create, edit, delete, move any note, and manage permissions. A user can be Owner on multiple unrelated subtrees. |
| **Writer** | Subtree | Can read, create, and edit any note within the subtree. Can delete or move only notes they authored. Cannot manage permissions or invite. |
| **Reader** | Subtree | Read-only access within the subtree. Cannot create, edit, delete, or move notes. Cannot manage permissions or invite. |

**Default-deny:** If no explicit grant exists for a user anywhere in a note's ancestry chain, access is denied. There is no "workspace default role" — the absence of a grant means no access.

**Key constraints:**

- Root Owner is singular and determined at workspace creation. It can be transferred to an existing peer (§6.2). It cannot be shared.
- Owner, Writer, and Reader are assigned per-note (subtree root) and inherit downward through the tree.
- A user can hold different roles on different subtrees (e.g., Owner on "Project Alpha", Reader on "Company Wiki", no access to "Sarah's Drafts").
- Authorship is determined by the identity that signed the `CreateNote` operation. This is immutable and cryptographically verifiable. It is used to evaluate Writer delete/move rights.

---

## 2. Permission Inheritance

### 2.1 Tree-Walk Resolution

The workspace is a forest — multiple root nodes with `parent_id = NULL` can exist. Permissions are set on individual notes and inherited by all descendants.

When checking whether user X has permission on note N:

1. Start at note N. Check for an explicit permission entry for user X.
2. If none found, walk up via `parent_id` to the parent note. Check again.
3. Repeat until an explicit entry is found or a root node is reached (with no match).
4. If no explicit entry is found anywhere in the tree, **access is denied** (default-deny).

The walk stops at the first explicit entry found.

**The Root Owner bypasses the tree walk entirely.** The RBAC gate recognizes the Root Owner identity and grants all capabilities without consulting the permission table. This is the only identity-based shortcut in the system.

### 2.2 Overriding Inherited Roles

A more specific (lower in tree) explicit entry overrides an inherited one. If Dave is Writer on Project Alpha but has an explicit Owner entry on the Architecture subtree, the Owner entry wins for Architecture and its descendants.

### 2.3 Example

```
Workspace (forest)
├─ Company Wiki          (bob: writer, carol: reader)
│  ├─ Onboarding Guide                    ← bob: writer, carol: reader (inherited)
│  └─ API Docs                            ← bob: writer, carol: reader (inherited)
├─ Project Alpha         (bob: owner, dave: writer)
│  ├─ Sprint Notes                        ← both inherit
│  └─ Architecture       (dave: owner)    ← dave elevated here (explicit), bob still owner (inherited from Project Alpha)
└─ Sarah's Drafts        (sarah: owner)
   └─ Private Notes                       ← only sarah has access (default-deny for all others)
```

---

## 3. Operation-Type Permission Matrix

Every operation is checked against the actor's effective role, resolved via tree-walk (§2.1) or Root Owner identity check.

### 3.1 Note Operations

| Operation | Root Owner | Owner | Writer | Reader |
|---|---|---|---|---|
| Read note content | Yes | Yes | Yes | Yes |
| CreateNote (child) | Yes | Yes | Yes | No |
| UpdateNote (title, properties) | Yes | Yes | Yes | No |
| UpdateField | Yes | Yes | Yes | No |
| SetTags | Yes | Yes | Yes | No |
| AddAttachment / RemoveAttachment | Yes | Yes | Yes | No |
| DeleteNote | Yes | Yes | Own only | No |
| MoveNote | Yes | Yes | Own only | No |

**MoveNote special rule:** Requires the actor's role to permit the move on **both** the source parent (delete capability) and the destination parent (create capability). For a Writer, this means they must have authored the note being moved AND hold at least Writer on both subtrees.

**Writer delete with children:** When a Writer deletes a note they authored, the chosen delete strategy determines which additional notes are affected. `DeleteAll` requires the actor to hold delete capability on all descendant notes — a Writer can only `DeleteAll` if they authored every descendant. `PromoteChildren` only requires delete capability on the target note itself; children are reparented to the deleted note's parent, not deleted.

**RetractOperation (undo):** A user can only retract their own operations. An Owner can retract any operation on notes within their subtree. The Root Owner can retract any operation in the workspace.

### 3.2 Permission Operations

| Operation | Root Owner | Owner | Writer | Reader |
|---|---|---|---|---|
| SetPermission (on a note) | Yes | Yes (within subtree, up to Owner) | No | No |
| RevokePermission (on a note) | Yes | Yes (any grant within subtree, regardless of who issued it) | No | No |
| Invite to workspace | Yes | No | No | No |
| Invite to subtree | Yes | Yes (subtrees they own) | No | No |
| TransferRootOwnership | Yes | No | No | No |
| RemovePeer | Yes | No | No | No |

### 3.3 Workspace-Level Operations (Root Owner Only)

| Operation | Root Owner | Owner | Writer | Reader |
|---|---|---|---|---|
| Create root-level note | Yes | No | No | No |
| CreateUserScript | Yes | No | No | No |
| UpdateUserScript | Yes | No | No | No |
| DeleteUserScript | Yes | No | No | No |
| Export workspace archive (.krillnotes) | Yes | No | No | No |

**.swarm delta bundles are not gated by RBAC.** Every peer generates and receives .swarm bundles as part of normal sync — this is the core transport mechanism, not a privileged operation. The contents of those bundles are individually permission-checked on receipt, but bundle generation itself is available to all peers.

### 3.4 Revocation Rights

| Actor | Can revoke |
|---|---|
| Root Owner | Any grant in the workspace |
| Subtree Owner | Any grant within their subtree |
| Writer | Nothing |
| Reader | Nothing |

---

## 4. Invitation & Sub-Delegation

Invitation is a permission grant bundled with an identity exchange. The invitation handshake mechanics (key exchange, known-contact vs unknown-peer flows) are defined in the Swarm Sync Design. This section defines only what the RBAC model permits during invitation.

### 4.1 Who Can Invite

| Inviter | Can invite to | Max grantable role |
|---|---|---|
| Root Owner | Entire workspace (individual grants on each existing root note, inheriting to their descendants) | Owner |
| Root Owner | Any specific subtree | Owner |
| Subtree Owner | Any subtree they own | Owner |
| Writer | Cannot invite | — |
| Reader | Cannot invite | — |

**"Invite to entire workspace"** is not a single workspace-level grant (workspace-level scope is Root Owner only). It is implemented as individual grants on every existing root note at the time of invitation. New root notes created after the invitation do not automatically include the invitee.

### 4.2 Role-Capped Delegation

**Principle:** You can grant roles up to and including Owner, within your own permitted scope. No one can grant Root Owner.

- Root Owner can grant Owner, Writer, or Reader on any subtree.
- Subtree Owner can grant Owner, Writer, or Reader within their subtree.
- Root Owner can only be transferred (§6), never granted.

The role ordering for cap checks: Root Owner > Owner > Writer > Reader.

### 4.3 Sub-Delegation Chains

Permission grants form verifiable chains. Each `SetPermission` operation is signed by the granting identity:

```
Alice (root owner)  → "Bob is owner on /Project Alpha"
Bob (owner)         → "Carol is writer on /Project Alpha"
Carol (writer)      → cannot delegate (Writers can't invite or set permissions)
```

Every link is valid because the granter held at least the role they granted. The chain is verifiable locally by any peer without contacting a central authority.

### 4.4 Post-Invitation Permission Changes

After invitation, the inviter (or any Owner/Root Owner with authority on the subtree) can:

- **Elevate** the invitee's role (up to their own role level).
- **Demote** the invitee's role.
- **Revoke** the invitee's access entirely.

These are standard `SetPermission` / `RevokePermission` operations, not invitation-specific mechanics.

---

## 5. Revocation

### 5.1 Revocation Is Eventually Consistent

In a decentralized system, revocation cannot be instantaneous. When Alice revokes Bob's Writer role, the `RevokePermission` operation must propagate to all peers via .swarm bundles. Until a peer receives the revocation, it may accept operations from Bob that were generated after the revocation.

Operations from revoked users that post-date the revocation are flagged as **contested** rather than deleted. Three operation states exist: valid, rejected, contested. Contested operations are preserved for audit and human review.

### 5.2 Role Revocation vs Peer Removal

These are distinct operations:

**Role revocation** (`RevokePermission`) removes a specific role grant on a specific subtree. The peer remains a known identity in the workspace and retains any other grants they hold elsewhere. A user who is Owner on Project Alpha and Writer on Company Wiki can have their Writer on Company Wiki revoked while keeping their Owner on Project Alpha.

**Peer removal** (`RemovePeer`) ejects an identity from the workspace entirely:

1. All permission grants held by the peer are revoked.
2. All downstream grants issued by the peer are cascade-invalidated (§5.3).
3. Future .swarm bundles omit the removed peer from the recipients list.
4. The peer is cryptographically cut off from all future updates.

Only the **Root Owner** can remove a peer. Subtree Owners can revoke roles within their subtree but cannot eject someone from the workspace — the peer may have legitimate grants on other subtrees.

### 5.3 Chain Cascade

When a revocation breaks a sub-delegation chain, downstream grants are re-evaluated:

1. The `RevokePermission` is applied, removing the target's permission entry.
2. All `SetPermission` operations where `granted_by` matches the revoked identity are identified.
3. Each downstream grant is evaluated: does the granter still hold a sufficient role to have issued it? If not, the grant is invalidated entirely (not automatically downgraded to match the granter's new role).
4. This recurses — invalidating Carol's grant triggers re-evaluation of grants Carol issued.
5. Invalidated grants are preserved in the operation log for audit. Their effect on the working permission state is removed.

**Demotion vs. full revocation:** If a granter is demoted (e.g., Owner to Writer) rather than fully revoked, downstream grants they issued are still re-evaluated. Any grant that exceeds the granter's new role is invalidated. For example, if Bob is demoted from Owner to Writer, an Owner grant Bob issued is invalidated, but a Reader grant Bob issued remains valid (Writer ≥ Reader).

**Example:**

```
Alice (root owner) → "Bob is owner on /Project Alpha"
Bob (owner)        → "Carol is writer on /Project Alpha"
Bob (owner)        → "Dave is reader on /Project Alpha"

Alice revokes Bob's owner role:
  → Bob loses owner on /Project Alpha
  → Carol's writer grant (issued by Bob) is re-evaluated
    → Bob no longer holds owner → Carol's grant is invalidated
  → Dave's reader grant (issued by Bob) is re-evaluated
    → Bob no longer holds owner → Dave's grant is invalidated
```

### 5.4 Transport Encryption and Revocation

Transport encryption provides defense in depth. When a peer is removed from the workspace, or when their last remaining grant is revoked (leaving them with no access):

1. The revocation propagates to all peers via bundles.
2. Future .swarm bundles omit the revoked user from the recipients list.
3. The revoked user cannot decrypt new bundles, even if intercepted.
4. The revoked user's local database remains accessible, but they are cryptographically cut off from all future updates.

A role revocation that leaves the user with other active grants elsewhere does not affect transport — the user continues to receive .swarm bundles because they remain a participant in the workspace.

### 5.5 Conflict Edge Cases

The following conflict rules interact with RBAC because the permission gate must decide which operations to accept when concurrent mutations conflict.

**Delete vs. Edit:** If device A deletes a note while device B edits it, the delete wins by default. Edits to a deleted note are discarded during bundle application. The deleted note's data is retained in the operation log.

**Tree Move Conflicts:** If two devices move the same note to different parents, LWW (last-writer-wins) is applied as the working state but the conflict is flagged for user review. Cycle detection is performed before applying any tree move — a move that would create a cycle is rejected.

---

## 6. Root Owner

The Root Owner is the identity that created the workspace. It is the singular authority from which all other permissions flow.

### 6.1 Exclusive Privileges

These capabilities are available **only** to the Root Owner and cannot be delegated:

| Capability | Description |
|---|---|
| Create root-level notes | Add new root nodes to the workspace forest |
| Script governance | Create, update, delete Rhai scripts (global effect on all note types) |
| Export workspace archive | Export to .krillnotes file (strips operation logs and permissions — fully accessible to anyone who loads it into a new workspace) |
| Workspace-wide invitation | Invite a peer with grants on all existing root notes |
| Remove peer | Eject an identity from the workspace entirely |
| Transfer root ownership | Hand over Root Owner status to an existing peer |

### 6.2 Root Owner Transfer

Root ownership can be transferred under the following constraints:

- Only the current Root Owner can initiate the transfer.
- The recipient must be an existing peer in the workspace (has a `JoinWorkspace` operation in the log). Transfer is not available during invitation.
- The transfer is atomic — one operation, one new Root Owner.
- On transfer, the outgoing Root Owner is granted **Owner on each existing root note** individually. They become a regular peer with per-subtree grants.
- New root notes created after the transfer do not automatically include the outgoing Root Owner.
- The new Root Owner can subsequently revoke or adjust any of the outgoing Root Owner's grants.
- Grants previously issued by the outgoing Root Owner remain valid. Transfer does not trigger chain cascade re-evaluation — the grants were authorized at the time they were issued.

### 6.3 Continuity Considerations

The Root Owner identity is bound to a single Ed25519 keypair. If the Root Owner's device is permanently lost before a transfer is performed, workspace-level operations (scripts, export, root note creation, peer removal) are frozen. Owners can continue to manage their subtrees, and existing peers retain their access, but no new root-level structural changes can be made.

**Recommended practice:** Transfer root ownership proactively before any anticipated disruption, or maintain a secure backup of the Root Owner's identity keypair.

---

*End of Role-Based Access Control Specification*

*Krillnotes RBAC Specification v2.0*

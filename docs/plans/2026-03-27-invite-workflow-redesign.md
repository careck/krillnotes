# Invite Workflow Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Streamline the invite/onboard flow so role and channel are chosen upfront, workspace-level invites are removed, and the invitee gets full context before accepting.

**Architecture:** Rewrite the dialog chain — replace CreateInviteDialog + ImportInviteDialog with InviteWorkflow + AcceptInviteWorkflow. Add `offered_role` to invite wire format. Add channel tracking to received responses. Simplify OnboardPeerDialog to auto-apply role and honor response channel.

**Tech Stack:** Rust (krillnotes-core), Tauri v2 commands, React 19, TypeScript, Tailwind v4, i18next

**Design spec:** `docs/plans/2026-03-27-invite-workflow-redesign-design.md`

---

## File Structure

### New files
- `krillnotes-desktop/src/components/InviteWorkflow.tsx` — Bob's invite creation (role + expiry + channel in one step)
- `krillnotes-desktop/src/components/AcceptInviteWorkflow.tsx` — Alice's invite acceptance (import → review → respond)

### Modified files
- `krillnotes-core/src/core/invite.rs` — Add `offered_role` to InviteFile, InviteRecord, create_invite()
- `krillnotes-core/src/core/received_response.rs` — Add `offered_role`, `response_channel`, `relay_account_id`
- `krillnotes-core/src/core/accepted_invite.rs` — Add `offered_role`
- `krillnotes-desktop/src-tauri/src/commands/invites.rs` — Add `offered_role` to InviteInfo, InviteFileData, create_invite cmd
- `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs` — Add fields to ReceivedResponseInfo
- `krillnotes-desktop/src-tauri/src/commands/sync.rs` — Add `offered_role` + `relay_account_id` to share_invite_link; set response_channel in fetch_relay_invite_response
- `krillnotes-desktop/src-tauri/src/menu.rs` — Remove "Accept Invite" menu item
- `krillnotes-desktop/src-tauri/src/lib.rs` — Remove menu mapping for file_accept_invite
- `krillnotes-desktop/src/types.ts` — Add new fields to TS interfaces
- `krillnotes-desktop/src/components/OnboardPeerDialog.tsx` — Remove role picker, ChannelPicker, Later button; auto-apply role; honor response channel
- `krillnotes-desktop/src/components/InviteManagerDialog.tsx` — Remove create/share/upload buttons, initialScope prop
- `krillnotes-desktop/src/components/IdentityManagerDialog.tsx` — Add "Accept Invite" button
- `krillnotes-desktop/src/components/WorkspaceView.tsx` — Wire context menu to InviteWorkflow instead of InviteManagerDialog
- `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx` — Remove "Share Invite Link" footer button
- `krillnotes-desktop/src/components/AcceptedInvitesSection.tsx` — Show offered_role badge
- `krillnotes-desktop/src/App.tsx` — Remove showAcceptInvite state and ImportInviteDialog render
- `krillnotes-desktop/src/hooks/useMenuEvents.ts` — Remove accept-invite menu event handler
- `krillnotes-desktop/src/i18n/locales/*.json` — Update strings (all 7 languages)

### Deleted files
- `krillnotes-desktop/src/components/CreateInviteDialog.tsx`
- `krillnotes-desktop/src/components/ImportInviteDialog.tsx`
- `krillnotes-desktop/src/components/AcceptPeerDialog.tsx`
- `krillnotes-desktop/src/components/SwarmInviteDialog.tsx`

---

## Task 1: Add `offered_role` to Core Invite Structs

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs`

- [ ] **Step 1: Add `offered_role` field to `InviteRecord`**

In `InviteRecord` (around line 22), add the field after `scope_note_title`:

```rust
    #[serde(default)]
    pub offered_role: String,  // "owner" | "writer" | "reader"
```

`#[serde(default)]` ensures backward compatibility when loading old invite records that lack this field.

- [ ] **Step 2: Add `offered_role` field to `InviteFile`**

In `InviteFile` (around line 44), add the field after `scope_note_title` and before `signature`:

```rust
    #[serde(default)]
    pub offered_role: String,
```

This field is included in the canonical JSON that gets signed, so it cannot be tampered with in transit.

- [ ] **Step 3: Add `offered_role` parameter to `InviteManager::create_invite()`**

In `create_invite()` (around line 221), add `offered_role: String` to the method signature, and wire it into both the `InviteRecord` and `InviteFile` construction.

The signature becomes:
```rust
pub fn create_invite(
    &mut self,
    workspace_id: &str,
    workspace_name: &str,
    expires_in_days: Option<u32>,
    signing_key: &SigningKey,
    inviter_declared_name: &str,
    workspace_description: Option<String>,
    workspace_author_name: Option<String>,
    workspace_author_org: Option<String>,
    workspace_homepage_url: Option<String>,
    workspace_license: Option<String>,
    workspace_tags: Vec<String>,
    scope_note_id: Option<String>,
    scope_note_title: Option<String>,
    offered_role: String,
) -> Result<(InviteRecord, InviteFile)>
```

Set `offered_role` on both the `InviteRecord` and `InviteFile` structs in the method body.

- [ ] **Step 4: Fix all callers of `create_invite`**

Search for all call sites of `create_invite` in the codebase (both in core and in commands). Add the `offered_role` argument. For now, callers that don't have the value yet should pass `"writer".to_string()` as a default — these will be updated in later tasks when the Tauri commands are modified.

- [ ] **Step 5: Run core tests**

```bash
cargo test -p krillnotes-core
```

Expected: All tests pass. Fix any compilation errors from the new field.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "feat: add offered_role to InviteFile and InviteRecord"
```

---

## Task 2: Add Channel Tracking to ReceivedResponse

**Files:**
- Modify: `krillnotes-core/src/core/received_response.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs`

- [ ] **Step 1: Add fields to core `ReceivedResponse` struct**

In `ReceivedResponse` (around line 25 in `received_response.rs`), add after `scope_note_title`:

```rust
    #[serde(default)]
    pub offered_role: String,
    #[serde(default)]
    pub response_channel: String,          // "relay" | "file"
    #[serde(default)]
    pub relay_account_id: Option<String>,
```

- [ ] **Step 2: Add fields to `ReceivedResponseInfo` Tauri struct**

In `ReceivedResponseInfo` (around line 17 in `receive_poll.rs`), add after `scope_note_title`:

```rust
    pub offered_role: String,
    pub response_channel: String,
    pub relay_account_id: Option<String>,
```

- [ ] **Step 3: Update the conversion from `ReceivedResponse` → `ReceivedResponseInfo`**

Find where `ReceivedResponseInfo` is constructed from `ReceivedResponse` (in `list_received_responses` or similar) and wire through the new fields.

- [ ] **Step 4: Run tests**

```bash
cargo test -p krillnotes-core && cargo build -p krillnotes-desktop
```

Expected: Compiles and tests pass.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/received_response.rs krillnotes-desktop/src-tauri/src/commands/receive_poll.rs
git commit -m "feat: add offered_role and response_channel to ReceivedResponse"
```

---

## Task 3: Add `offered_role` to Tauri Invite Structs and Commands

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/invites.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs`

- [ ] **Step 1: Add `offered_role` to `InviteInfo`**

In `InviteInfo` (around line 12 in `invites.rs`), add:

```rust
    pub offered_role: String,
```

Update the conversion from `InviteRecord` → `InviteInfo` to wire this through.

- [ ] **Step 2: Add `offered_role`, `scope_note_id`, `scope_note_title` to `InviteFileData`**

In `InviteFileData` (around line 52 in `invites.rs`), add:

```rust
    pub offered_role: String,
    pub scope_note_id: Option<String>,
    pub scope_note_title: Option<String>,
```

Update the conversion from `InviteFile` → `InviteFileData` to wire these through. The invitee needs to see the role and subtree info when reviewing the invite.

- [ ] **Step 3: Add `offered_role` param to `create_invite` Tauri command**

In `create_invite` command (around line 96 in `invites.rs`), add `offered_role: String` parameter and pass it through to `InviteManager::create_invite()`.

- [ ] **Step 4: Add `offered_role` and `relay_account_id` params to `share_invite_link`**

In `share_invite_link` command (around line 227 in `sync.rs`), add:
- `offered_role: String` — pass through to `create_invite`
- `relay_account_id: Option<String>` — use to select which relay account uploads the invite (if `None`, use the existing logic that picks the first available account)

- [ ] **Step 5: Set `response_channel` in invite response import commands**

In `import_invite_response` (around line 305 in `invites.rs`):
- When creating the `ReceivedResponse` record, set `response_channel: "file".to_string()`
- Look up the `InviteRecord` by `invite_id` and copy `offered_role` to the `ReceivedResponse`

In `fetch_relay_invite_response` (around line 693 in `sync.rs`):
- When creating the `ReceivedResponse` record, set `response_channel: "relay".to_string()`
- Set `relay_account_id` to the relay account that was used to fetch
- Look up the `InviteRecord` by `invite_id` and copy `offered_role`

- [ ] **Step 6: Build and verify**

```bash
cargo build -p krillnotes-desktop
```

Expected: Compiles successfully.

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/invites.rs krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat: wire offered_role through Tauri invite commands"
```

---

## Task 4: Add `offered_role` to AcceptedInvite

**Files:**
- Modify: `krillnotes-core/src/core/accepted_invite.rs`
- Modify: Tauri command file where `AcceptedInviteInfo` is defined (find via grep)

- [ ] **Step 1: Add `offered_role` to core `AcceptedInvite`**

In `AcceptedInvite` (around line 23 in `accepted_invite.rs`), add:

```rust
    #[serde(default)]
    pub offered_role: String,
```

- [ ] **Step 2: Add `offered_role` to `AcceptedInviteInfo` Tauri struct**

Find the Tauri-serializable `AcceptedInviteInfo` struct (likely in `commands/invites.rs` or `commands/receive_poll.rs`) and add:

```rust
    pub offered_role: String,
```

Wire through in the conversion.

- [ ] **Step 3: Set `offered_role` when saving accepted invite**

In the `save_accepted_invite` command (or wherever `AcceptedInvite` records are created), add `offered_role` parameter and persist it.

- [ ] **Step 4: Build and verify**

```bash
cargo build -p krillnotes-desktop
```

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/accepted_invite.rs krillnotes-desktop/src-tauri/src/commands/
git commit -m "feat: add offered_role to AcceptedInvite"
```

---

## Task 5: Update TypeScript Types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

- [ ] **Step 1: Add `offeredRole` to `InviteInfo`**

In the `InviteInfo` interface (around line 272), add:

```typescript
  offeredRole: string;
```

- [ ] **Step 2: Add fields to `InviteFileData`**

In the `InviteFileData` interface (around line 297), add:

```typescript
  offeredRole: string;
  scopeNoteId: string | null;
  scopeNoteTitle: string | null;
```

- [ ] **Step 3: Add fields to `ReceivedResponseInfo`**

In the `ReceivedResponseInfo` interface (around line 339), add:

```typescript
  offeredRole: string;
  responseChannel: "relay" | "file";
  relayAccountId: string | null;
```

- [ ] **Step 4: Add `offeredRole` to `AcceptedInviteInfo`**

In the `AcceptedInviteInfo` interface (around line 326), add:

```typescript
  offeredRole: string;
```

- [ ] **Step 5: Run TypeScript type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: Type errors in components that now need to handle the new fields. These will be fixed in subsequent tasks.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add offeredRole and channel fields to TS types"
```

---

## Task 6: Create InviteWorkflow Component

**Files:**
- Create: `krillnotes-desktop/src/components/InviteWorkflow.tsx`

This replaces `CreateInviteDialog.tsx`. Single dialog with: subtree (read-only), role picker, expiry picker, channel toggle (relay with account picker / file), submit → success screen.

- [ ] **Step 1: Create `InviteWorkflow.tsx` with props and state**

```typescript
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import type { InviteInfo, RelayAccountInfo } from "../types";

interface Props {
  identityUuid: string;
  workspaceName: string;
  scopeNoteId: string;
  scopeNoteTitle: string;
  onCreated: (invite: InviteInfo) => void;
  onClose: () => void;
}

type Step = "configure" | "success";
type Channel = "relay" | "file";

export default function InviteWorkflow({
  identityUuid,
  workspaceName,
  scopeNoteId,
  scopeNoteTitle,
  onCreated,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>("configure");
  const [role, setRole] = useState<"owner" | "writer" | "reader">("writer");
  const [expiryDays, setExpiryDays] = useState<number | null>(30);
  const [customDays, setCustomDays] = useState("");
  const [channel, setChannel] = useState<Channel>("relay");
  const [relayAccounts, setRelayAccounts] = useState<RelayAccountInfo[]>([]);
  const [selectedRelayId, setSelectedRelayId] = useState("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [relayUrl, setRelayUrl] = useState<string | null>(null);
  const [createdInvite, setCreatedInvite] = useState<InviteInfo | null>(null);

  useEffect(() => {
    invoke<RelayAccountInfo[]>("list_relay_accounts", { identityUuid }).then(
      (accounts) => {
        setRelayAccounts(accounts);
        if (accounts.length === 1) {
          setSelectedRelayId(accounts[0].accountId);
        }
        if (accounts.length === 0) {
          setChannel("file");
        }
      }
    );
  }, [identityUuid]);

  // ... render below
}
```

- [ ] **Step 2: Implement the configure step UI**

Add the render method for step === "configure":

- Subtree display (read-only, shows `scopeNoteTitle`)
- Role dropdown (Owner / Writer / Reader, default Writer)
- Expiry dropdown (No expiry / 7 days / 30 days / Custom with number input)
- Channel toggle: two styled cards for Relay and File
  - Relay card disabled with hint if `relayAccounts.length === 0`
  - When relay selected, show relay account dropdown below (email @ server)
- Submit button: "Create & Copy Link" (relay) or "Create & Save" (file)
- Cancel button

Reference the existing `CreateInviteDialog.tsx` for styling patterns (Tailwind classes, dialog structure). Use the same dialog overlay/backdrop pattern.

- [ ] **Step 3: Implement relay submit handler**

```typescript
async function handleSubmitRelay() {
  setCreating(true);
  setError(null);
  try {
    const days = expiryDays === -1 ? Number(customDays) : expiryDays;
    const invite = await invoke<InviteInfo>("share_invite_link", {
      identityUuid,
      workspaceName,
      expiresInDays: days,
      scopeNoteId,
      offeredRole: role,
      relayAccountId: selectedRelayId || null,
    });
    if (invite.relayUrl) {
      await writeText(invite.relayUrl);
      setRelayUrl(invite.relayUrl);
    }
    setCreatedInvite(invite);
    onCreated(invite);
    setStep("success");
  } catch (e) {
    setError(String(e));
  } finally {
    setCreating(false);
  }
}
```

- [ ] **Step 4: Implement file submit handler**

```typescript
async function handleSubmitFile() {
  const savePath = await save({
    defaultPath: `${workspaceName}-invite.swarm`,
    filters: [{ name: "Swarm Invite", extensions: ["swarm"] }],
  });
  if (!savePath) return;
  setCreating(true);
  setError(null);
  try {
    const days = expiryDays === -1 ? Number(customDays) : expiryDays;
    const invite = await invoke<InviteInfo>("create_invite", {
      identityUuid,
      workspaceName,
      expiresInDays: days,
      savePath,
      scopeNoteId,
      offeredRole: role,
    });
    setCreatedInvite(invite);
    onCreated(invite);
    setStep("success");
  } catch (e) {
    setError(String(e));
  } finally {
    setCreating(false);
  }
}
```

- [ ] **Step 5: Implement success step UI**

For relay: show checkmark, relay URL (monospace, copyable), summary of role/subtree/expiry, "Copy Again" and "Done" buttons.

For file: show file saved confirmation, summary, "Done" button.

- [ ] **Step 6: Verify manually**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: Compiles. The component is not wired in yet — that happens in Task 11.

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src/components/InviteWorkflow.tsx
git commit -m "feat: create InviteWorkflow component (replaces CreateInviteDialog)"
```

---

## Task 7: Create AcceptInviteWorkflow Component

**Files:**
- Create: `krillnotes-desktop/src/components/AcceptInviteWorkflow.tsx`

This replaces `ImportInviteDialog.tsx`. Three-step dialog: import (URL or file) → review (role, subtree, inviter, metadata) → respond (channel picker with inline relay signup).

- [ ] **Step 1: Create `AcceptInviteWorkflow.tsx` with props, state, and step management**

```typescript
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import type { InviteFileData, FetchedRelayInvite, RelayAccountInfo } from "../types";

interface Props {
  identityUuid: string;
  identityName: string;
  onResponded: () => void;
  onClose: () => void;
}

type Step = "import" | "review" | "respond";

export default function AcceptInviteWorkflow({
  identityUuid,
  identityName,
  onResponded,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>("import");

  // Import state
  const [relayUrl, setRelayUrl] = useState("");
  const [fetchingRelay, setFetchingRelay] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Invite data (populated after import)
  const [inviteData, setInviteData] = useState<InviteFileData | null>(null);
  const [inviteTempPath, setInviteTempPath] = useState<string | null>(null);
  const [inviteRelayServer, setInviteRelayServer] = useState<string | null>(null);

  // Respond state
  const [relayAccounts, setRelayAccounts] = useState<RelayAccountInfo[]>([]);
  const [responseChannel, setResponseChannel] = useState<"relay" | "file">("relay");
  const [sending, setSending] = useState(false);
  const [responseRelayUrl, setResponseRelayUrl] = useState<string | null>(null);

  // Inline signup state
  const [showSignup, setShowSignup] = useState(false);
  const [signupEmail, setSignupEmail] = useState("");
  const [signupPassword, setSignupPassword] = useState("");
  const [signingUp, setSigningUp] = useState(false);

  // ... handlers and render below
}
```

- [ ] **Step 2: Implement Step 1 — Import**

Two import paths:

**Relay URL import:**
```typescript
async function handleFetchRelay() {
  setFetchingRelay(true);
  setError(null);
  try {
    // Extract token and base URL from the pasted URL
    const url = new URL(relayUrl.trim());
    const pathParts = url.pathname.split("/");
    const token = pathParts[pathParts.length - 1];
    const relayBaseUrl = `${url.protocol}//${url.host}`;

    const result = await invoke<FetchedRelayInvite>("fetch_relay_invite", {
      token,
      relayBaseUrl,
    });
    setInviteData(result.invite);
    setInviteTempPath(result.tempPath);
    setInviteRelayServer(url.host);
    setStep("review");
  } catch (e) {
    setError(String(e));
  } finally {
    setFetchingRelay(false);
  }
}
```

**File import:**
```typescript
async function handleLoadFile() {
  const path = await open({
    filters: [{ name: "Swarm Invite", extensions: ["swarm"] }],
  });
  if (!path) return;
  setError(null);
  try {
    const data = await invoke<InviteFileData>("import_invite", {
      path: typeof path === "string" ? path : path.path,
    });
    setInviteData(data);
    setInviteTempPath(typeof path === "string" ? path : path.path);
    setInviteRelayServer(null); // file import — no relay server
    setStep("review");
  } catch (e) {
    setError(String(e));
  }
}
```

Render: identity label at top ("Accepting as: {identityName}"), URL text input + "Fetch" button, "or" divider, "Load .swarm file" button.

- [ ] **Step 3: Implement Step 2 — Review**

Display the invite details from `inviteData`:

**Primary section:**
- Invited by: `inviteData.inviterDeclaredName` + truncated fingerprint (`inviteData.inviterFingerprint`)
- Role: badge showing `inviteData.offeredRole` (colored pill — green for writer, blue for reader, purple for owner)
- Subtree: `inviteData.scopeNoteTitle` (if present)
- Relay server: `inviteRelayServer` (if invite came via relay)
- Expires: formatted `inviteData.expiresAt` or "No expiry"

**Collapsible section (workspace info):**
Use a `<details>` element:
- Description: `inviteData.workspaceDescription`
- Author: `inviteData.workspaceAuthorName`, `inviteData.workspaceAuthorOrg`
- Homepage: `inviteData.workspaceHomepageUrl`
- License: `inviteData.workspaceLicense`
- Tags: `inviteData.workspaceTags`

Buttons: "Decline" (closes dialog) and "Next" (advances to respond step).

On "Next", load relay accounts:
```typescript
async function handleNextToRespond() {
  const accounts = await invoke<RelayAccountInfo[]>("list_relay_accounts", { identityUuid });
  setRelayAccounts(accounts);

  // Pre-select relay if invite came via relay
  if (inviteRelayServer) {
    const match = accounts.find((a) => a.relayUrl.includes(inviteRelayServer!));
    if (match) {
      setResponseChannel("relay");
    } else {
      // No account on inviter's relay — show signup prompt
      setShowSignup(true);
      setResponseChannel("relay");
    }
  } else if (accounts.length > 0) {
    setResponseChannel("relay");
  } else {
    setResponseChannel("file");
  }
  setStep("respond");
}
```

- [ ] **Step 4: Implement Step 3 — Respond**

**Channel picker UI:**
- Relay card (highlighted if `inviteRelayServer` matches an account):
  - If matching account: show "✓ account@server" with checkmark
  - If no match but `inviteRelayServer` set: show inline signup form (email + password + "Create account & respond" button)
  - If no relay server info: show generic relay account dropdown
- File card (always available)

**Inline relay signup handler:**
```typescript
async function handleSignupAndRespond() {
  setSigningUp(true);
  setError(null);
  try {
    await invoke("register_relay_account", {
      identityUuid,
      relayUrl: `https://${inviteRelayServer}`,
      email: signupEmail,
      password: signupPassword,
    });
    // Refresh accounts and proceed with relay response
    const accounts = await invoke<RelayAccountInfo[]>("list_relay_accounts", { identityUuid });
    setRelayAccounts(accounts);
    setShowSignup(false);
    await handleSendViaRelay();
  } catch (e) {
    setError(String(e));
  } finally {
    setSigningUp(false);
  }
}
```

**Relay response handler:**
```typescript
async function handleSendViaRelay() {
  setSending(true);
  setError(null);
  try {
    const url = await invoke<string>("send_invite_response_via_relay", {
      identityUuid,
      tempPath: inviteTempPath,
      expiresInDays: 30,
    });
    setResponseRelayUrl(url);
    // Save accepted invite record
    await invoke("save_accepted_invite", {
      identityUuid,
      inviteId: inviteData!.inviteId,
      workspaceId: inviteData!.workspaceId,
      workspaceName: inviteData!.workspaceName,
      inviterPublicKey: inviteData!.inviterPublicKey,
      inviterDeclaredName: inviteData!.inviterDeclaredName,
      responseRelayUrl: url,
      offeredRole: inviteData!.offeredRole,
    });
    onResponded();
  } catch (e) {
    setError(String(e));
  } finally {
    setSending(false);
  }
}
```

**File response handler:**
```typescript
async function handleSendViaFile() {
  const savePath = await save({
    defaultPath: `${inviteData!.workspaceName}-response.swarm`,
    filters: [{ name: "Swarm Response", extensions: ["swarm"] }],
  });
  if (!savePath) return;
  setSending(true);
  setError(null);
  try {
    await invoke("respond_to_invite", {
      identityUuid,
      invitePath: inviteTempPath,
      savePath,
    });
    await invoke("save_accepted_invite", {
      identityUuid,
      inviteId: inviteData!.inviteId,
      workspaceId: inviteData!.workspaceId,
      workspaceName: inviteData!.workspaceName,
      inviterPublicKey: inviteData!.inviterPublicKey,
      inviterDeclaredName: inviteData!.inviterDeclaredName,
      responseRelayUrl: null,
      offeredRole: inviteData!.offeredRole,
    });
    onResponded();
  } catch (e) {
    setError(String(e));
  } finally {
    setSending(false);
  }
}
```

Buttons: "Decline" and "Accept & Send".

- [ ] **Step 5: Type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/AcceptInviteWorkflow.tsx
git commit -m "feat: create AcceptInviteWorkflow component (replaces ImportInviteDialog)"
```

---

## Task 8: Simplify OnboardPeerDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/OnboardPeerDialog.tsx`

- [ ] **Step 1: Remove role picker state and UI**

Remove:
- `role` state variable (around line 26) — replace with `response.offeredRole` usage
- The `<select>` role picker UI (around lines 177-192)
- Replace with a read-only role badge:

```tsx
<div className="text-sm">
  <span className="text-secondary">{t('invite.role')}</span>
  <span className={`ml-2 px-2 py-0.5 rounded text-xs font-medium ${
    response.offeredRole === 'owner' ? 'bg-purple-500/20 text-purple-300' :
    response.offeredRole === 'writer' ? 'bg-green-500/20 text-green-300' :
    'bg-blue-500/20 text-blue-300'
  }`}>
    {t(`roles.${response.offeredRole}`)}
  </span>
</div>
```

- [ ] **Step 2: Remove ChannelPicker and replace with read-only channel display**

Remove:
- `channelType` state variable (around line 27)
- `relayAccounts` state variable (around line 28)
- `selectedRelayId` state variable (around line 29)
- The `useEffect` that loads relay accounts
- The `<ChannelPicker>` component usage (around lines 199-206)
- The ChannelPicker import

Replace with a read-only display:

```tsx
<div className="text-sm">
  <span className="text-secondary">{t('invite.channel')}</span>
  <span className="ml-2">
    {response.responseChannel === 'relay' ? '🔗 Relay' : '💾 File'}
  </span>
</div>
```

- [ ] **Step 3: Remove "Later" button**

Remove the "Later" button and its `handleLater` function. Only "Reject" and "Grant & Sync" remain.

- [ ] **Step 4: Update `handleGrantAndSync` to use invite role and response channel**

Replace the role variable usage with `response.offeredRole`:

```typescript
// In set_permission call:
await invoke("set_permission", {
  noteId: response.scopeNoteId,
  userId: response.inviteePublicKey,
  role: response.offeredRole,  // was: role
});
```

Replace channel logic:

```typescript
// Send snapshot based on response channel
if (response.responseChannel === "relay") {
  await invoke("send_snapshot_via_relay", {
    identityUuid,
    peerPublicKeys: [response.inviteePublicKey],
  });
} else {
  // File channel — prompt for save location
  const savePath = await save({
    defaultPath: `${response.workspaceName}-snapshot.swarm`,
    filters: [{ name: "Swarm Snapshot", extensions: ["swarm"] }],
  });
  if (!savePath) {
    setProcessing(false);
    return;
  }
  await invoke("create_snapshot_for_peers", {
    identityUuid,
    peerPublicKeys: [response.inviteePublicKey],
    savePath,
  });
}
```

- [ ] **Step 5: Type check and verify**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/OnboardPeerDialog.tsx
git commit -m "feat: simplify OnboardPeerDialog — auto-apply role, honor response channel"
```

---

## Task 9: Strip InviteManagerDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/InviteManagerDialog.tsx`

- [ ] **Step 1: Remove `initialScope` prop and auto-open logic**

Remove `initialScope` from the `Props` interface (around line 9-14). Remove the `useEffect` that auto-opens CreateInviteDialog when `initialScope` is set (around lines 49-53). Remove `showCreate` state variable.

- [ ] **Step 2: Remove creation buttons**

Remove:
- "+ Create Invite" button (around lines 242-247)
- "Share Invite Link" button (around lines 248-254) and the `handleShareInviteLink` function
- Associated state variables: `sharingLink`, `shareError`, `shareSuccess`, `showRelaySetup`, `pendingShareAction`

- [ ] **Step 3: Remove "Upload to Relay" per-invite button**

Remove the "Upload to Relay" button from each invite row (around lines 336-344) and the `handleUploadToRelay` function. Remove `uploadingRelayFor` state.

- [ ] **Step 4: Remove CreateInviteDialog import and render**

Remove the `import CreateInviteDialog` statement and the `{showCreate && <CreateInviteDialog ... />}` render block.

- [ ] **Step 5: Add `offeredRole` display to invite list rows**

In each invite row, after the scope display, add the role badge:

```tsx
{invite.offeredRole && (
  <span className={`px-2 py-0.5 rounded text-xs font-medium ${
    invite.offeredRole === 'owner' ? 'bg-purple-500/20 text-purple-300' :
    invite.offeredRole === 'writer' ? 'bg-green-500/20 text-green-300' :
    'bg-blue-500/20 text-blue-300'
  }`}>
    {t(`roles.${invite.offeredRole}`)}
  </span>
)}
```

- [ ] **Step 6: Type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src/components/InviteManagerDialog.tsx
git commit -m "feat: strip InviteManagerDialog to list-only (no create buttons)"
```

---

## Task 10: Add "Accept Invite" to Identity Dialog

**Files:**
- Modify: `krillnotes-desktop/src/components/IdentityManagerDialog.tsx`

- [ ] **Step 1: Add state for AcceptInviteWorkflow**

```typescript
const [showAcceptInvite, setShowAcceptInvite] = useState(false);
```

- [ ] **Step 2: Add "Accept Invite" button to the identity action bar**

In the action buttons section for the selected identity (around lines 504-523), add a button that only shows when the identity is unlocked:

```tsx
{selectedUuid && unlockedIds.has(selectedUuid) && (
  <button
    onClick={() => setShowAcceptInvite(true)}
    className="px-3 py-1.5 text-sm rounded-md border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
  >
    {t('invite.acceptInvite')}
  </button>
)}
```

- [ ] **Step 3: Render AcceptInviteWorkflow dialog**

Import and render the new component:

```tsx
import AcceptInviteWorkflow from "./AcceptInviteWorkflow";

// In the render, after the existing dialogs:
{showAcceptInvite && selectedUuid && (
  <AcceptInviteWorkflow
    identityUuid={selectedUuid}
    identityName={identities.find(i => i.uuid === selectedUuid)?.name ?? ""}
    onResponded={() => {
      setShowAcceptInvite(false);
      // Refresh accepted invites section
    }}
    onClose={() => setShowAcceptInvite(false)}
  />
)}
```

- [ ] **Step 4: Type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/IdentityManagerDialog.tsx
git commit -m "feat: add Accept Invite button to Identity dialog"
```

---

## Task 11: Wire Context Menu to InviteWorkflow + Update WorkspaceView

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

- [ ] **Step 1: Change inviteScope handler to open InviteWorkflow instead of InviteManagerDialog**

The `inviteScope` state (around line 87) stays but now triggers InviteWorkflow instead of InviteManagerDialog.

Replace the InviteManagerDialog render block (around lines 878-882):

```tsx
// Old: InviteManagerDialog with initialScope
// New: InviteWorkflow directly
{inviteScope && workspaceInfo.identityUuid && (
  <InviteWorkflow
    identityUuid={workspaceInfo.identityUuid}
    workspaceName={workspaceInfo.filename}
    scopeNoteId={inviteScope.noteId}
    scopeNoteTitle={inviteScope.noteTitle}
    onCreated={() => {}}
    onClose={() => setInviteScope(null)}
  />
)}
```

Import `InviteWorkflow` and remove the `InviteManagerDialog` import (if it was only used here — check if it's still imported for the peers dialog).

- [ ] **Step 2: Type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: wire context menu to InviteWorkflow instead of InviteManagerDialog"
```

---

## Task 12: Remove File Menu "Accept Invite" + App.tsx Cleanup

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src/App.tsx`

- [ ] **Step 1: Remove menu item from `menu.rs`**

Remove the `accept_invite_item` construction (around lines 153-157) and its addition to the File submenu (around line 166).

- [ ] **Step 2: Remove menu mapping from `lib.rs`**

Remove the `("file_accept_invite", "File > Accept Invite clicked")` entry from the `MENU_MESSAGES` array (around line 109).

- [ ] **Step 3: Remove handler from `useMenuEvents.ts`**

In the useMenuEvents hook file (likely at `krillnotes-desktop/src/hooks/useMenuEvents.ts`), remove the `'File > Accept Invite clicked'` handler entry.

- [ ] **Step 4: Remove state and render from `App.tsx`**

Remove `setShowAcceptInvite` from the `useMenuEvents` call arguments. Remove the `showAcceptInvite` state variable, the `ImportInviteDialog` render block that it triggers, and the `ImportInviteDialog` import. Also remove `showSwarmInvite` / `setShowSwarmInvite` state and `SwarmInviteDialog` render if still present (SwarmInviteDialog is being deleted).

- [ ] **Step 5: Remove from locale strings in `menu.rs` locales**

Check if `acceptInvite` is used in `build.rs` or locale JSON files for menu strings and remove it.

- [ ] **Step 6: Build and verify**

```bash
cd krillnotes-desktop && cargo build -p krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src/App.tsx krillnotes-desktop/src/hooks/
git commit -m "feat: remove Accept Invite from File menu (now in Identity dialog)"
```

---

## Task 13: Clean Up WorkspacePeersDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

- [ ] **Step 1: Remove "Share Invite Link" footer button**

Remove the "Share Invite Link" button (around lines 463-469), the `handleShareInviteLink` function, and associated state: `sharingLink`, `shareError`, `shareSuccess`, `showRelaySetup`, `pendingShareAction`.

- [ ] **Step 2: Type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspacePeersDialog.tsx
git commit -m "feat: remove Share Invite Link from WorkspacePeersDialog"
```

---

## Task 14: Update AcceptedInvitesSection to Show Role

**Files:**
- Modify: `krillnotes-desktop/src/components/AcceptedInvitesSection.tsx`

- [ ] **Step 1: Add role badge to each accepted invite**

In the invite list rendering, add a role badge after the workspace name or inviter name:

```tsx
{invite.offeredRole && (
  <span className={`ml-2 px-2 py-0.5 rounded text-xs font-medium ${
    invite.offeredRole === 'owner' ? 'bg-purple-500/20 text-purple-300' :
    invite.offeredRole === 'writer' ? 'bg-green-500/20 text-green-300' :
    'bg-blue-500/20 text-blue-300'
  }`}>
    {t(`roles.${invite.offeredRole}`)}
  </span>
)}
```

- [ ] **Step 2: Type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/AcceptedInvitesSection.tsx
git commit -m "feat: show offered role badge in AcceptedInvitesSection"
```

---

## Task 15: Delete Old Components

**Files:**
- Delete: `krillnotes-desktop/src/components/CreateInviteDialog.tsx`
- Delete: `krillnotes-desktop/src/components/ImportInviteDialog.tsx`
- Delete: `krillnotes-desktop/src/components/AcceptPeerDialog.tsx`
- Delete: `krillnotes-desktop/src/components/SwarmInviteDialog.tsx`

- [ ] **Step 1: Search for remaining imports of deleted components**

```bash
grep -rn "CreateInviteDialog\|ImportInviteDialog\|AcceptPeerDialog\|SwarmInviteDialog" krillnotes-desktop/src/
```

Remove any remaining imports or references. If any component still imports a deleted file, update it.

- [ ] **Step 2: Delete the files**

```bash
rm krillnotes-desktop/src/components/CreateInviteDialog.tsx
rm krillnotes-desktop/src/components/ImportInviteDialog.tsx
rm krillnotes-desktop/src/components/AcceptPeerDialog.tsx
rm krillnotes-desktop/src/components/SwarmInviteDialog.tsx
```

- [ ] **Step 3: Build and type check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: No import errors. All references have been cleaned up.

- [ ] **Step 4: Commit**

```bash
git add -A krillnotes-desktop/src/components/
git commit -m "chore: delete replaced invite dialog components"
```

---

## Task 16: Update i18n Strings

**Files:**
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`
- Modify: All other locale files (de.json, es.json, fr.json, ja.json, pt.json, zh.json)

- [ ] **Step 1: Add new English strings**

Add to the `"invite"` namespace in `en.json`:

```json
"role": "Role",
"channel": "Channel",
"configureInvite": "Invite to Subtree",
"selectRole": "Select role",
"selectChannel": "How to send",
"viaRelay": "Relay",
"viaFile": "File",
"relayHint": "Copy a shareable link",
"fileHint": "Save a .swarm file",
"noRelayAccounts": "No relay accounts configured",
"createAndCopyLink": "Create & Copy Link",
"createAndSave": "Create & Save",
"inviteLinkCopied": "Invite link copied!",
"copyAgain": "Copy Again",
"done": "Done",
"acceptingAs": "Accepting as: {{name}}",
"pasteRelayUrl": "Paste relay invite URL...",
"loadSwarmFile": "Load .swarm file",
"inviteDetails": "Invite Details",
"workspaceInfo": "Workspace info",
"relayServer": "Relay server",
"respondVia": "Respond via",
"noAccountOnServer": "No account on {{server}}",
"createAccountAndRespond": "Create account & respond",
"acceptAndSend": "Accept & Send",
"decline": "Decline",
"fileSaved": "File saved",
"next": "Next",
"inviterRelay": "Inviter's relay"
```

- [ ] **Step 2: Remove obsolete strings**

Remove strings for deleted components if they are no longer referenced:
- `"createTitle"`, `"createDescription"` (if only used by CreateInviteDialog)
- `"acceptTitle"` (if only used by AcceptPeerDialog)
- `"shareInviteLink"` and `"sharing"` (if only used by removed buttons)

Check each string is truly unused before removing.

- [ ] **Step 3: Update other locale files**

Copy the new key structure to all 6 other locale files. Leave values in English for now — the existing pattern in the codebase uses English as fallback, and translations can be done separately.

- [ ] **Step 4: Remove menu locale strings**

Remove the `"acceptInvite"` key from the menu locale files (the ones embedded via `build.rs`). Check `krillnotes-desktop/src-tauri/locales/` or wherever menu strings are defined.

- [ ] **Step 5: Type check and build**

```bash
cd krillnotes-desktop && npx tsc --noEmit && cargo build -p krillnotes-desktop
```

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/i18n/ krillnotes-desktop/src-tauri/locales/
git commit -m "feat: update i18n strings for invite workflow redesign"
```

---

## Task 17: Full Integration Test

- [ ] **Step 1: Build everything**

```bash
cd krillnotes-desktop && npm update && npm run tauri dev
```

- [ ] **Step 2: Test Bob's invite flow**

1. Open a workspace with an identity
2. Right-click a note → "Invite to subtree"
3. Verify InviteWorkflow opens with: subtree name, role picker (Owner/Writer/Reader), expiry picker, channel toggle
4. Select "Writer" role, "30 days" expiry
5. If relay account exists: select Relay, pick account, click "Create & Copy Link"
6. Verify success screen shows link + summary
7. If no relay: select File, click "Create & Save", verify OS save dialog

- [ ] **Step 3: Test invite list**

1. Open Workspace Peers → Manage Invites
2. Verify no "+ Create Invite" or "Share Invite Link" buttons
3. Verify invite list shows role badge alongside scope
4. Verify "Copy Link", "Revoke", "Delete" actions work

- [ ] **Step 4: Test Alice's accept flow**

1. Open Identity Manager, select unlocked identity
2. Verify "Accept Invite" button appears
3. Click it → verify AcceptInviteWorkflow opens
4. Paste relay URL or load .swarm file
5. Verify review screen shows: role, subtree, inviter, expiry, relay server (if relay), collapsible workspace metadata
6. Click "Next" → verify channel picker pre-selects inviter's relay
7. If no account on server → verify inline signup form appears
8. Click "Accept & Send" → verify response is sent

- [ ] **Step 5: Test onboarding**

1. As Bob, check Pending Responses section
2. Click "Onboard" on a response
3. Verify OnboardPeerDialog shows: peer info, role badge (read-only, matches invite), channel display (read-only, matches response channel)
4. No role picker, no ChannelPicker, no "Later" button
5. Click "Grant & Sync" → verify permission is set with correct role and snapshot is sent via correct channel

- [ ] **Step 6: Verify Accept Invite is gone from File menu**

1. Check File menu — "Accept Invite" should not be present

- [ ] **Step 7: Commit final state**

```bash
git add -A
git commit -m "feat: complete invite workflow redesign (#113)"
```

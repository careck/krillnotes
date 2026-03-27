// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
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

const EXPIRY_OPTIONS = [
  { label: "No expiry", value: null },
  { label: "7 days", value: 7 },
  { label: "30 days", value: 30 },
  { label: "Custom", value: -1 },
];

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
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    invoke<RelayAccountInfo[]>("list_relay_accounts", { identityUuid }).then(
      (accounts) => {
        setRelayAccounts(accounts);
        if (accounts.length === 1) {
          setSelectedRelayId(accounts[0].relayAccountId);
        }
        if (accounts.length === 0) {
          setChannel("file");
        }
      }
    );
  }, [identityUuid]);

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
        try { await navigator.clipboard.writeText(invite.relayUrl); } catch { /* WKWebView fallback — URL shown below */ }
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

  async function handleCopyAgain() {
    if (!relayUrl) return;
    try {
      await navigator.clipboard.writeText(relayUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch { /* WKWebView fallback — URL is visible above */ }
  }

  function formatExpiryLabel(invite: InviteInfo): string {
    if (!invite.expiresAt) return t("invite.noExpiry", "No expiry");
    const d = new Date(invite.expiresAt);
    return d.toLocaleDateString();
  }

  // ── Success step ──────────────────────────────────────────────────────────

  if (step === "success" && createdInvite) {
    return (
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70">
        <div className="bg-background border border-border rounded-xl shadow-xl p-6 w-full max-w-md">
          {/* Checkmark / heading */}
          <div className="flex items-center gap-3 mb-4">
            <div className="flex-shrink-0 w-9 h-9 rounded-full bg-green-100 dark:bg-green-900/40 flex items-center justify-center">
              <svg
                className="w-5 h-5 text-green-600 dark:text-green-400"
                fill="none"
                stroke="currentColor"
                strokeWidth={2.5}
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M5 13l4 4L19 7"
                />
              </svg>
            </div>
            <h2 className="text-lg font-semibold">
              {channel === "relay"
                ? t("invite.successTitleRelay", "Invite link created")
                : t("invite.successTitleFile", "Invite file saved")}
            </h2>
          </div>

          {/* Relay URL display */}
          {channel === "relay" && relayUrl && (
            <div className="mb-4">
              <p className="text-xs text-muted-foreground mb-1">
                {t("invite.relayLinkLabel", "Link (copied to clipboard)")}
              </p>
              <div className="flex items-center gap-2">
                <p className="text-xs font-mono bg-secondary rounded px-3 py-2 break-all flex-1 select-all">
                  {relayUrl}
                </p>
                <button
                  onClick={handleCopyAgain}
                  className="flex-shrink-0 px-3 py-2 text-xs rounded border border-border hover:bg-secondary"
                >
                  {copied
                    ? t("invite.copied", "Copied!")
                    : t("invite.copyAgain", "Copy again")}
                </button>
              </div>
            </div>
          )}

          {/* File saved message */}
          {channel === "file" && (
            <p className="text-sm text-muted-foreground mb-4">
              {t(
                "invite.fileSavedMessage",
                "The invite file has been saved. Share it with the person you want to invite."
              )}
            </p>
          )}

          {/* Summary */}
          <div className="bg-secondary rounded-lg px-4 py-3 mb-5 space-y-1.5 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">
                {t("invite.summarySubtree", "Subtree")}
              </span>
              <span className="font-medium">{scopeNoteTitle}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">
                {t("invite.summaryRole", "Role")}
              </span>
              <span className="font-medium capitalize">
                {createdInvite.offeredRole}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">
                {t("invite.summaryExpiry", "Expires")}
              </span>
              <span className="font-medium">
                {formatExpiryLabel(createdInvite)}
              </span>
            </div>
          </div>

          {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

          <div className="flex justify-end">
            <button
              onClick={onClose}
              className="px-4 py-2 text-sm rounded bg-primary text-primary-foreground"
            >
              {t("common.done", "Done")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  // ── Configure step ────────────────────────────────────────────────────────

  const canSubmit =
    !creating &&
    (expiryDays !== -1 || parseInt(customDays) > 0) &&
    (channel === "file" || selectedRelayId !== "");

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70">
      <div className="bg-background border border-border rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-4">
          {t("invite.createTitle", "Create invite")}
        </h2>

        {/* Subtree (read-only) */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">
            {t("invite.subtreeLabel", "Subtree")}
          </label>
          <p className="text-sm bg-secondary rounded px-3 py-1.5 text-foreground">
            {scopeNoteTitle}
          </p>
        </div>

        {/* Role */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">
            {t("invite.roleLabel", "Role")}
          </label>
          <select
            className="w-full border border-border rounded px-3 py-2 bg-background"
            value={role}
            onChange={(e) =>
              setRole(e.target.value as "owner" | "writer" | "reader")
            }
          >
            <option value="owner">{t("invite.roleOwner", "Owner")}</option>
            <option value="writer">{t("invite.roleWriter", "Writer")}</option>
            <option value="reader">{t("invite.roleReader", "Reader")}</option>
          </select>
        </div>

        {/* Expiry */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">
            {t("invite.expiry", "Expiry")}
          </label>
          <select
            className="w-full border border-border rounded px-3 py-2 bg-background"
            value={expiryDays ?? "null"}
            onChange={(e) =>
              setExpiryDays(
                e.target.value === "null" ? null : parseInt(e.target.value)
              )
            }
          >
            {EXPIRY_OPTIONS.map((opt) => (
              <option key={String(opt.value)} value={String(opt.value)}>
                {opt.label}
              </option>
            ))}
          </select>
          {expiryDays === -1 && (
            <div className="mt-2">
              <label className="block text-sm font-medium mb-1">
                {t("invite.customDays", "Days")}
              </label>
              <input
                type="number"
                min="1"
                className="w-full border border-border rounded px-3 py-2 bg-background"
                value={customDays}
                onChange={(e) => setCustomDays(e.target.value)}
                placeholder="e.g. 14"
              />
            </div>
          )}
        </div>

        {/* Channel selection */}
        <div className="mb-5">
          <label className="block text-sm font-medium mb-2">
            {t("invite.channelLabel", "Delivery channel")}
          </label>
          <div className="grid grid-cols-2 gap-3">
            {/* Relay card */}
            <button
              type="button"
              disabled={relayAccounts.length === 0}
              onClick={() => setChannel("relay")}
              className={[
                "flex flex-col items-start rounded-lg border px-4 py-3 text-left transition-colors",
                channel === "relay" && relayAccounts.length > 0
                  ? "border-primary bg-primary/5"
                  : "border-border",
                relayAccounts.length === 0
                  ? "opacity-50 cursor-not-allowed"
                  : "hover:bg-secondary cursor-pointer",
              ].join(" ")}
            >
              <span className="text-sm font-medium mb-0.5">
                {t("invite.channelRelay", "Relay link")}
              </span>
              <span className="text-xs text-muted-foreground">
                {relayAccounts.length === 0
                  ? t(
                      "invite.channelRelayDisabled",
                      "No relay accounts configured"
                    )
                  : t(
                      "invite.channelRelayHint",
                      "Copy a shareable link to clipboard"
                    )}
              </span>
            </button>

            {/* File card */}
            <button
              type="button"
              onClick={() => setChannel("file")}
              className={[
                "flex flex-col items-start rounded-lg border px-4 py-3 text-left transition-colors cursor-pointer",
                channel === "file"
                  ? "border-primary bg-primary/5"
                  : "border-border hover:bg-secondary",
              ].join(" ")}
            >
              <span className="text-sm font-medium mb-0.5">
                {t("invite.channelFile", "File")}
              </span>
              <span className="text-xs text-muted-foreground">
                {t("invite.channelFileHint", "Save a .swarm file to share")}
              </span>
            </button>
          </div>

          {/* Relay account selector */}
          {channel === "relay" && relayAccounts.length > 0 && (
            <div className="mt-3">
              <label className="block text-sm font-medium mb-1">
                {t("invite.relayAccountLabel", "Relay account")}
              </label>
              <select
                className="w-full border border-border rounded px-3 py-2 bg-background"
                value={selectedRelayId}
                onChange={(e) => setSelectedRelayId(e.target.value)}
              >
                {relayAccounts.length > 1 && (
                  <option value="">
                    {t("invite.selectRelayAccount", "Select an account…")}
                  </option>
                )}
                {relayAccounts.map((acct) => {
                  const server = new URL(acct.relayUrl).hostname;
                  return (
                    <option key={acct.relayAccountId} value={acct.relayAccountId}>
                      {acct.email} @ {server}
                    </option>
                  );
                })}
              </select>
            </div>
          )}
        </div>

        {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm rounded border border-border hover:bg-secondary"
          >
            {t("common.cancel", "Cancel")}
          </button>
          <button
            onClick={channel === "relay" ? handleSubmitRelay : handleSubmitFile}
            disabled={!canSubmit}
            className="px-4 py-2 text-sm rounded bg-primary text-primary-foreground disabled:opacity-50"
          >
            {creating
              ? t("common.saving", "Saving…")
              : channel === "relay"
              ? t("invite.createAndCopyLink", "Create & Copy Link")
              : t("invite.createAndSave", "Create & Save")}
          </button>
        </div>
      </div>
    </div>
  );
}

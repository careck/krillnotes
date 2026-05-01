import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { InviteFileData, RelayAccountInfo, FetchedRelayInvite } from '../types';

interface Props {
  identityUuid: string;
  identityName: string;
  onResponded: () => void;
  onClose: () => void;
  // Optional: pre-loaded invite from file-drop
  preloadedInviteData?: InviteFileData;
  preloadedPath?: string;
}

type Step = 'import' | 'review' | 'respond';

export function AcceptInviteWorkflow({ identityUuid, identityName, onResponded, onClose, preloadedInviteData, preloadedPath }: Props) {
  const { t } = useTranslation();

  // Step management
  const [step, setStep] = useState<Step>('import');

  // Import state
  const [relayUrl, setRelayUrl] = useState('');
  const [fetchingRelay, setFetchingRelay] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Invite data
  const [inviteData, setInviteData] = useState<InviteFileData | null>(null);
  const [inviteTempPath, setInviteTempPath] = useState<string | null>(null);
  const [inviteRelayServer, setInviteRelayServer] = useState<string | null>(null);

  // Respond state
  const [relayAccounts, setRelayAccounts] = useState<RelayAccountInfo[]>([]);
  const [responseChannel, setResponseChannel] = useState<'relay' | 'file'>('file');
  const [sending, setSending] = useState(false);
  const [responseRelayUrl, setResponseRelayUrl] = useState<string | null>(null);

  // Inline signup state
  const [showSignup, setShowSignup] = useState(false);
  const [signupEmail, setSignupEmail] = useState('');
  const [signupPassword, setSignupPassword] = useState('');
  const [signingUp, setSigningUp] = useState(false);

  // If pre-loaded data is provided (file-drop path), skip straight to the review step
  useEffect(() => {
    if (preloadedInviteData && preloadedPath) {
      setInviteData(preloadedInviteData);
      setInviteTempPath(preloadedPath);
      setInviteRelayServer(null);
      setStep('review');
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Step 1: Import ──────────────────────────────────────────────────────────

  async function handleFetchRelay() {
    setFetchingRelay(true);
    setError(null);
    try {
      const url = new URL(relayUrl.trim());
      const pathParts = url.pathname.split('/');
      const token = pathParts[pathParts.length - 1];

      const result = await invoke<FetchedRelayInvite>('fetch_relay_invite', { token, relayBaseUrl: url.origin });
      setInviteData(result.invite);
      setInviteTempPath(result.tempPath);
      setInviteRelayServer(url.host);
      setStep('review');
    } catch (e) {
      setError(String(e));
    } finally {
      setFetchingRelay(false);
    }
  }

  async function handleLoadFile() {
    const path = await open({
      filters: [{ name: 'Swarm Invite', extensions: ['swarm'] }],
    });
    if (!path) return;
    setError(null);
    try {
      const filePath = typeof path === 'string' ? path : (path as { path: string }).path;
      const data = await invoke<InviteFileData>('import_invite', { path: filePath });
      setInviteData(data);
      setInviteTempPath(filePath);
      setInviteRelayServer(null);
      setStep('review');
    } catch (e) {
      setError(String(e));
    }
  }

  // ── Step 2: Review → Respond transition ────────────────────────────────────

  async function handleNextToRespond() {
    const accounts = await invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid });
    setRelayAccounts(accounts);

    if (inviteRelayServer) {
      const match = accounts.find((a) => a.relayUrl.includes(inviteRelayServer!));
      if (match) {
        setResponseChannel('relay');
      } else {
        setShowSignup(true);
        setResponseChannel('relay');
      }
    } else if (accounts.length > 0) {
      setResponseChannel('relay');
    } else {
      setResponseChannel('file');
    }
    setStep('respond');
  }

  // ── Step 3: Respond ─────────────────────────────────────────────────────────

  async function handleSendViaRelay() {
    setSending(true);
    setError(null);
    try {
      const url = await invoke<string>('send_invite_response_via_relay', {
        identityUuid,
        tempPath: inviteTempPath,
        expiresInDays: 30,
      });
      setResponseRelayUrl(url);
      try { navigator.clipboard.writeText(url); } catch { /* fallback: URL is shown in UI */ }
      await invoke('save_accepted_invite', {
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

  async function handleSignupAndRespond() {
    setSigningUp(true);
    setError(null);
    try {
      await invoke('register_relay_account', {
        identityUuid,
        relayUrl: `https://${inviteRelayServer}`,
        email: signupEmail,
        password: signupPassword,
      });
      const accounts = await invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid });
      setRelayAccounts(accounts);
      setShowSignup(false);
      await handleSendViaRelay();
    } catch (e) {
      setError(String(e));
    } finally {
      setSigningUp(false);
    }
  }

  async function handleSendViaFile() {
    const savePath = await save({
      defaultPath: `${inviteData!.workspaceName}-response.swarm`,
      filters: [{ name: 'Swarm Response', extensions: ['swarm'] }],
    });
    if (!savePath) return;
    setSending(true);
    setError(null);
    try {
      await invoke('respond_to_invite', {
        identityUuid,
        invitePath: inviteTempPath,
        savePath,
      });
      await invoke('save_accepted_invite', {
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

  // ── Helpers ─────────────────────────────────────────────────────────────────

  function roleBadge(role: string) {
    const cls =
      role === 'owner'
        ? 'bg-purple-100 text-purple-800'
        : role === 'writer'
        ? 'bg-green-100 text-green-800'
        : 'bg-blue-100 text-blue-800';
    return (
      <span className={`text-xs font-medium px-2 py-0.5 rounded-full ${cls}`}>
        {role}
      </span>
    );
  }

  function truncateFingerprint(fp: string) {
    if (fp.length <= 16) return fp;
    return `${fp.slice(0, 8)}…${fp.slice(-8)}`;
  }

  function formatExpiry(expiresAt: string | null) {
    if (!expiresAt) return t('invite.noExpiry', 'No expiry');
    return new Date(expiresAt).toLocaleDateString();
  }

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70">
      <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg shadow-xl p-6 w-full max-w-lg">

        {/* Header */}
        <h2 className="text-lg font-semibold mb-1">
          {t('invite.acceptInvite', 'Accept Invite')}
        </h2>
        <p className="text-xs text-[var(--color-muted-foreground)] mb-4">
          {t('invite.acceptingAs', 'Accepting as')}: <span className="font-medium">{identityName}</span>
        </p>

        {/* Step indicators */}
        <div className="flex items-center gap-2 mb-5">
          {(['import', 'review', 'respond'] as Step[]).map((s, i) => (
            <div key={s} className="flex items-center gap-2">
              {i > 0 && <div className="w-6 h-px bg-[var(--color-border)]" />}
              <div
                className={`flex items-center gap-1.5 text-xs ${
                  step === s
                    ? 'text-blue-600 font-medium'
                    : i < ['import', 'review', 'respond'].indexOf(step)
                    ? 'text-[var(--color-muted-foreground)]'
                    : 'text-[var(--color-muted-foreground)] opacity-50'
                }`}
              >
                <span
                  className={`w-5 h-5 rounded-full flex items-center justify-center text-xs font-medium border ${
                    step === s
                      ? 'bg-blue-600 text-white border-blue-600'
                      : i < ['import', 'review', 'respond'].indexOf(step)
                      ? 'border-[var(--color-border)] bg-[var(--color-secondary)]'
                      : 'border-[var(--color-border)]'
                  }`}
                >
                  {i + 1}
                </span>
                {s === 'import'
                  ? t('invite.stepImport', 'Import')
                  : s === 'review'
                  ? t('invite.stepReview', 'Review')
                  : t('invite.stepRespond', 'Respond')}
              </div>
            </div>
          ))}
        </div>

        {/* ── Step 1: Import ── */}
        {step === 'import' && (
          <div className="space-y-3">
            <div className="p-3 border border-[var(--color-border)] rounded-md bg-[var(--color-secondary)]/30">
              <label className="block text-xs font-medium text-[var(--color-muted-foreground)] mb-1">
                {t('invite.relayUrlLabel', 'Paste a relay invite URL')}
              </label>
              <div className="flex gap-2">
                <input
                  type="url"
                  value={relayUrl}
                  onChange={(e) => setRelayUrl(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter' && relayUrl.trim()) handleFetchRelay(); }}
                  placeholder="https://swarm.krillnotes.org/invites/..."
                  className="flex-1 border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                  disabled={fetchingRelay}
                />
                <button
                  onClick={handleFetchRelay}
                  disabled={!relayUrl.trim() || fetchingRelay}
                  className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
                >
                  {fetchingRelay ? t('common.loading', 'Loading…') : t('invite.fetchRelay', 'Fetch')}
                </button>
              </div>

              <div className="flex items-center gap-3 my-3">
                <div className="flex-1 border-t border-[var(--color-border)]" />
                <span className="text-xs text-[var(--color-muted-foreground)]">{t('common.or', 'or')}</span>
                <div className="flex-1 border-t border-[var(--color-border)]" />
              </div>

              <button
                onClick={handleLoadFile}
                className="w-full px-3 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
              >
                {t('invite.openSwarmFile', 'Load .swarm Invite File…')}
              </button>
            </div>

            {error && <p className="text-red-500 text-sm">{error}</p>}

            <div className="flex justify-end">
              <button
                onClick={onClose}
                className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
              >
                {t('common.cancel')}
              </button>
            </div>
          </div>
        )}

        {/* ── Step 2: Review ── */}
        {step === 'review' && inviteData && (
          <div className="space-y-3">
            {/* Primary invite details */}
            <div className="p-4 border border-[var(--color-border)] rounded-md space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-xs text-[var(--color-muted-foreground)]">
                  {t('invite.invitedBy', 'Invited by')}
                </span>
              </div>
              <div className="flex items-start gap-2">
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium">{inviteData.inviterDeclaredName}</p>
                  <p className="text-xs font-mono text-[var(--color-muted-foreground)] truncate">
                    {truncateFingerprint(inviteData.inviterFingerprint)}
                  </p>
                </div>
              </div>

              <div className="flex items-center gap-2 pt-1">
                <span className="text-xs text-[var(--color-muted-foreground)]">{t('invite.role', 'Role')}:</span>
                {roleBadge(inviteData.offeredRole)}
              </div>

              {inviteData.scopeNoteTitle && (
                <div className="flex items-center gap-2">
                  <span className="text-xs text-[var(--color-muted-foreground)]">{t('invite.subtree', 'Subtree')}:</span>
                  <span className="text-xs font-medium">{inviteData.scopeNoteTitle}</span>
                </div>
              )}

              {inviteRelayServer && (
                <div className="flex items-center gap-2">
                  <span className="text-xs text-[var(--color-muted-foreground)]">{t('invite.relayServer', 'Relay')}:</span>
                  <span className="text-xs font-mono">{inviteRelayServer}</span>
                </div>
              )}

              <div className="flex items-center gap-2">
                <span className="text-xs text-[var(--color-muted-foreground)]">{t('invite.expires', 'Expires')}:</span>
                <span className="text-xs">{formatExpiry(inviteData.expiresAt)}</span>
              </div>
            </div>

            {/* Collapsible workspace info */}
            <details className="border border-[var(--color-border)] rounded-md">
              <summary className="px-3 py-2 text-sm font-medium cursor-pointer select-none hover:bg-[var(--color-secondary)]/50">
                {inviteData.workspaceName}
              </summary>
              <div className="px-3 pb-3 pt-1 space-y-1">
                {inviteData.workspaceDescription && (
                  <p className="text-sm text-[var(--color-muted-foreground)]">{inviteData.workspaceDescription}</p>
                )}
                {inviteData.workspaceAuthorName && (
                  <p className="text-xs text-[var(--color-muted-foreground)]">
                    {t('invite.by', 'By')} {inviteData.workspaceAuthorName}
                    {inviteData.workspaceAuthorOrg && ` (${inviteData.workspaceAuthorOrg})`}
                  </p>
                )}
                {inviteData.workspaceHomepageUrl && (
                  <p className="text-xs text-[var(--color-muted-foreground)]">
                    {t('invite.homepage', 'Homepage')}: {inviteData.workspaceHomepageUrl}
                  </p>
                )}
                {inviteData.workspaceLicense && (
                  <p className="text-xs text-[var(--color-muted-foreground)]">
                    {t('invite.license', 'License')}: {inviteData.workspaceLicense}
                  </p>
                )}
                {inviteData.workspaceTags.length > 0 && (
                  <div className="flex flex-wrap gap-1 pt-1">
                    {inviteData.workspaceTags.map((tag) => (
                      <span key={tag} className="text-xs bg-[var(--color-secondary)] px-2 py-0.5 rounded-full">
                        {tag}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            </details>

            {inviteData.expiresAt && new Date(inviteData.expiresAt) < new Date() && (
              <p className="text-red-500 text-sm">{t('invite.expired', 'This invite has expired.')}</p>
            )}

            {error && <p className="text-red-500 text-sm">{error}</p>}

            <div className="flex justify-end gap-2">
              <button
                onClick={onClose}
                className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
              >
                {t('invite.decline', 'Decline')}
              </button>
              <button
                onClick={handleNextToRespond}
                className="px-4 py-2 text-sm rounded bg-blue-600 text-white"
              >
                {t('common.next', 'Next')}
              </button>
            </div>
          </div>
        )}

        {/* ── Step 3: Respond ── */}
        {step === 'respond' && inviteData && (
          <div className="space-y-3">
            <p className="text-sm text-[var(--color-muted-foreground)]">
              {t('invite.chooseResponseChannel', 'Choose how to send your response:')}
            </p>

            {/* Relay card */}
            <div
              className={`p-3 border rounded-md cursor-pointer transition-colors ${
                responseChannel === 'relay'
                  ? 'border-blue-500 bg-blue-50/30'
                  : 'border-[var(--color-border)] hover:bg-[var(--color-secondary)]/30'
              }`}
              onClick={() => setResponseChannel('relay')}
            >
              <div className="flex items-start gap-2">
                <input
                  type="radio"
                  checked={responseChannel === 'relay'}
                  onChange={() => setResponseChannel('relay')}
                  className="mt-0.5"
                />
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium">{t('invite.viaRelay', 'Via relay')}</p>
                  {inviteRelayServer && (() => {
                    const match = relayAccounts.find((a) => a.relayUrl.includes(inviteRelayServer!));
                    if (match) {
                      return (
                        <p className="text-xs text-green-600 mt-0.5">
                          ✓ {match.email}@{inviteRelayServer}
                        </p>
                      );
                    }
                    return (
                      <p className="text-xs text-[var(--color-muted-foreground)] mt-0.5">
                        {t('invite.noRelayAccount', 'No account on')} {inviteRelayServer}
                      </p>
                    );
                  })()}
                  {!inviteRelayServer && relayAccounts.length > 0 && (
                    <p className="text-xs text-[var(--color-muted-foreground)] mt-0.5">
                      {relayAccounts[0].email} — {relayAccounts[0].relayUrl}
                    </p>
                  )}
                </div>
              </div>

              {/* Inline signup form */}
              {showSignup && responseChannel === 'relay' && inviteRelayServer && (
                <div className="mt-3 pt-3 border-t border-[var(--color-border)] space-y-2">
                  <p className="text-xs font-medium text-[var(--color-muted-foreground)]">
                    {t('invite.createRelayAccount', 'Create account on')} {inviteRelayServer}
                  </p>
                  <input
                    type="email"
                    value={signupEmail}
                    onChange={(e) => setSignupEmail(e.target.value)}
                    placeholder={t('common.email', 'Email')}
                    className="w-full border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                    disabled={signingUp}
                  />
                  <input
                    type="password"
                    value={signupPassword}
                    onChange={(e) => setSignupPassword(e.target.value)}
                    placeholder={t('common.password', 'Password')}
                    className="w-full border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                    disabled={signingUp}
                  />
                  <button
                    onClick={handleSignupAndRespond}
                    disabled={signingUp || !signupEmail.trim() || !signupPassword.trim()}
                    className="w-full px-3 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
                  >
                    {signingUp
                      ? t('common.saving', 'Saving…')
                      : t('invite.createAccountAndRespond', 'Create account & respond')}
                  </button>
                </div>
              )}
            </div>

            {/* File card */}
            <div
              className={`p-3 border rounded-md cursor-pointer transition-colors ${
                responseChannel === 'file'
                  ? 'border-blue-500 bg-blue-50/30'
                  : 'border-[var(--color-border)] hover:bg-[var(--color-secondary)]/30'
              }`}
              onClick={() => setResponseChannel('file')}
            >
              <div className="flex items-start gap-2">
                <input
                  type="radio"
                  checked={responseChannel === 'file'}
                  onChange={() => setResponseChannel('file')}
                  className="mt-0.5"
                />
                <div>
                  <p className="text-sm font-medium">{t('invite.viaFile', 'Save response file')}</p>
                  <p className="text-xs text-[var(--color-muted-foreground)] mt-0.5">
                    {t('invite.viaFileDesc', 'Save a .swarm file and share it with the inviter.')}
                  </p>
                </div>
              </div>
            </div>

            {error && <p className="text-red-500 text-sm">{error}</p>}

            <div className="flex justify-end gap-2">
              <button
                onClick={onClose}
                className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
              >
                {t('invite.decline', 'Decline')}
              </button>
              <button
                onClick={responseChannel === 'relay' && !showSignup ? handleSendViaRelay : responseChannel === 'file' ? handleSendViaFile : undefined}
                disabled={sending || signingUp || (responseChannel === 'relay' && showSignup)}
                className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
              >
                {sending
                  ? t('common.saving', 'Saving…')
                  : t('invite.acceptAndSend', 'Accept & Send')}
              </button>
            </div>
          </div>
        )}

        {/* Success: relay URL returned */}
        {responseRelayUrl && (
          <div className="mt-3 p-3 bg-green-50/50 border border-green-200 rounded-md">
            <p className="text-xs text-green-700 mb-1">{t('invite.responseShared', 'Response sent. Share this link with the inviter:')}</p>
            <p className="text-xs font-mono break-all text-[var(--color-muted-foreground)] p-2 bg-[var(--color-secondary)]/50 rounded">
              {responseRelayUrl}
            </p>
            <button
              onClick={() => { try { navigator.clipboard.writeText(responseRelayUrl); } catch { /* fallback */ } }}
              className="mt-2 px-3 py-1 text-xs rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
            >
              {t('invite.copyLink', 'Copy link')}
            </button>
          </div>
        )}

      </div>
    </div>
  );
}

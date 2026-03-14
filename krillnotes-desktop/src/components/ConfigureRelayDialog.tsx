// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { RelayInfo } from '../types';

interface Props {
  identityUuid: string;
  peerDeviceId: string;
  onClose: () => void;
  onConfigured: () => void;
}

type Tab = 'register' | 'login';

function mapError(raw: string): string {
  const s = raw.toLowerCase();
  if (s.includes('identity') && (s.includes('lock') || s.includes('unlock')))
    return 'Please unlock your identity before configuring relay.';
  if (s.includes('http 409') || s.includes('email_exists') || s.includes('already exists'))
    return 'Email already registered on this relay — use the Login tab instead.';
  // Fall back to the raw error so the user always sees what went wrong.
  return raw;
}

export default function ConfigureRelayDialog({
  identityUuid,
  peerDeviceId,
  onClose,
  onConfigured,
}: Props) {
  const [activeTab, setActiveTab] = useState<Tab>('register');
  const [relayUrl, setRelayUrl] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [initialising, setInitialising] = useState(true);

  // On mount: check if credentials are already stored to pre-fill and pick tab.
  useEffect(() => {
    invoke<RelayInfo | null>('get_relay_info')
      .then(info => {
        if (info) {
          setRelayUrl(info.relayUrl);
          setEmail(info.email);
          setActiveTab('login');
        }
      })
      .catch(() => {/* ignore — fall back to register tab */})
      .finally(() => setInitialising(false));
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (activeTab === 'register' && password !== confirmPassword) {
      setError('Passwords do not match.');
      return;
    }
    if (!relayUrl.trim() || !email.trim() || !password) {
      setError('All fields are required.');
      return;
    }

    setLoading(true);
    try {
      if (activeTab === 'register') {
        await invoke('configure_relay', { identityUuid, relayUrl, email, password });
      } else {
        await invoke('relay_login', { identityUuid, relayUrl, email, password });
      }
      // Ensure the peer is marked as relay channel, storing the URL in
      // channelParams for display/reference (relay routing uses disk credentials,
      // not this field, but the integration tests and peer display use it).
      await invoke('update_peer_channel', {
        peerDeviceId,
        channelType: 'relay',
        channelParams: JSON.stringify({ relay_url: relayUrl }),
      });
      onConfigured();
    } catch (err) {
      setError(mapError(String(err)));
    } finally {
      setLoading(false);
    }
  };

  const inputClass =
    'w-full px-3 py-1.5 text-sm rounded border border-[var(--color-border)] ' +
    'bg-[var(--color-background)] text-[var(--color-foreground)] ' +
    'focus:outline-none focus:ring-1 focus:ring-blue-500';

  return (
    <div className="fixed inset-0 z-70 flex items-center justify-center bg-black/50">
      <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg shadow-xl w-[420px] flex flex-col">

        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-[var(--color-border)]">
          <h2 className="text-base font-semibold">Configure Relay</h2>
          <button
            onClick={onClose}
            className="text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)] px-2"
          >
            ✕
          </button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-[var(--color-border)]">
          {(['register', 'login'] as Tab[]).map(tab => (
            <button
              key={tab}
              onClick={() => { setActiveTab(tab); setError(null); }}
              className={
                'flex-1 py-2 text-sm font-medium capitalize ' +
                (activeTab === tab
                  ? 'border-b-2 border-blue-500 text-[var(--color-foreground)]'
                  : 'text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]')
              }
            >
              {tab}
            </button>
          ))}
        </div>

        {/* Form */}
        {initialising ? (
          <p className="p-6 text-sm text-center text-[var(--color-muted-foreground)]">Loading…</p>
        ) : (
          <form onSubmit={handleSubmit} className="p-4 space-y-3">
            <div>
              <label className="block text-xs font-medium mb-1">Relay URL</label>
              <input
                type="url"
                value={relayUrl}
                onChange={e => setRelayUrl(e.target.value)}
                placeholder="https://relay.example.com"
                className={inputClass}
                required
              />
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">Email</label>
              <input
                type="email"
                value={email}
                onChange={e => setEmail(e.target.value)}
                placeholder="you@example.com"
                className={inputClass}
                required
              />
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">Password</label>
              <input
                type="password"
                value={password}
                onChange={e => setPassword(e.target.value)}
                className={inputClass}
                required
              />
            </div>
            {activeTab === 'register' && (
              <div>
                <label className="block text-xs font-medium mb-1">Confirm Password</label>
                <input
                  type="password"
                  value={confirmPassword}
                  onChange={e => setConfirmPassword(e.target.value)}
                  className={inputClass}
                  required
                />
              </div>
            )}

            {error && (
              <p className="text-xs text-red-500 bg-red-500/10 px-3 py-2 rounded">
                {error}
              </p>
            )}

            <div className="flex justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={onClose}
                className="px-3 py-1.5 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={loading}
                className="px-3 py-1.5 text-sm font-medium bg-blue-600 text-white rounded hover:bg-blue-700 disabled:opacity-50"
              >
                {loading
                  ? (activeTab === 'register' ? 'Registering…' : 'Logging in…')
                  : (activeTab === 'register' ? 'Register' : 'Log in')}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}

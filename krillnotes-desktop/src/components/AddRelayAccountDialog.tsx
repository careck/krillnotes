import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';

interface Props {
  identityUuid: string;
  onClose: () => void;
  onCreated: () => void;
}

type Tab = 'register' | 'login';

function mapError(raw: string): string {
  const s = raw.toLowerCase();
  if (s.includes('identity') && (s.includes('lock') || s.includes('unlock')))
    return 'Please unlock your identity before configuring relay.';
  if (s.includes('http 409') || s.includes('email_exists') || s.includes('already exists'))
    return 'Email already registered on this relay — use the Login tab instead.';
  return raw;
}

export default function AddRelayAccountDialog({ identityUuid, onClose, onCreated }: Props) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<Tab>('register');
  const [relayUrl, setRelayUrl] = useState('https://swarm.krillnotes.org');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (activeTab === 'register' && password !== confirmPassword) {
      setError(t('addRelay.passwordMismatch'));
      return;
    }
    if (!relayUrl.trim() || !email.trim() || !password) {
      setError('All fields are required.');
      return;
    }

    setLoading(true);
    try {
      if (activeTab === 'register') {
        await invoke('register_relay_account', { identityUuid, relayUrl, email, password });
      } else {
        await invoke('login_relay_account', { identityUuid, relayUrl, email, password });
      }
      onCreated();
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
          <h2 className="text-base font-semibold">{t('addRelay.title')}</h2>
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
                'flex-1 py-2 text-sm font-medium ' +
                (activeTab === tab
                  ? 'border-b-2 border-blue-500 text-[var(--color-foreground)]'
                  : 'text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]')
              }
            >
              {tab === 'register' ? t('addRelay.register') : t('addRelay.login')}
            </button>
          ))}
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="p-4 space-y-3">
          <div>
            <label className="block text-xs font-medium mb-1">{t('addRelay.relayUrl')}</label>
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
            <label className="block text-xs font-medium mb-1">{t('addRelay.email')}</label>
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
            <label className="block text-xs font-medium mb-1">{t('addRelay.password')}</label>
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
              <label className="block text-xs font-medium mb-1">{t('addRelay.confirmPassword')}</label>
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
              {t('common.cancel')}
            </button>
            <button
              type="submit"
              disabled={loading}
              className="px-3 py-1.5 text-sm font-medium bg-blue-600 text-white rounded hover:bg-blue-700 disabled:opacity-50"
            >
              {loading
                ? (activeTab === 'register' ? t('addRelay.register') + '...' : t('addRelay.login') + '...')
                : t('addRelay.submit')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

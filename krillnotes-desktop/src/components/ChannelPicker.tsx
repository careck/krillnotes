// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useTranslation } from 'react-i18next';
import type { RelayAccountInfo } from '../types';

export type ChannelType = 'relay' | 'folder' | 'manual';

interface ChannelPickerProps {
  selectedType: ChannelType;
  onTypeChange: (type: ChannelType) => void;
  relayAccounts: RelayAccountInfo[];
  selectedRelayAccountId?: string;
  onRelayAccountSelect?: (accountId: string) => void;
  currentFolderPath?: string | null;
  onConfigureFolder?: () => void;
  disabled?: boolean;
}

export function ChannelPicker({
  selectedType,
  onTypeChange,
  relayAccounts,
  selectedRelayAccountId,
  onRelayAccountSelect,
  currentFolderPath,
  onConfigureFolder,
  disabled,
}: ChannelPickerProps) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1.5">
        <select
          value={selectedType}
          onChange={e => onTypeChange(e.target.value as ChannelType)}
          disabled={disabled}
          className="text-xs px-1.5 py-0.5 rounded border border-[var(--color-border)] bg-[var(--color-background)] text-[var(--color-foreground)]"
        >
          <option value="relay">Relay</option>
          <option value="folder">Folder</option>
          <option value="manual">Manual</option>
        </select>

        {selectedType === 'folder' && onConfigureFolder && (
          <button
            onClick={onConfigureFolder}
            disabled={disabled}
            className="text-xs px-2 py-0.5 rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)] disabled:opacity-50"
          >
            {t('peers.configure', 'Configure')}
          </button>
        )}
      </div>

      {selectedType === 'relay' && (
        relayAccounts.length === 0 ? (
          <p className="text-xs text-[var(--color-muted-foreground)] italic mt-0.5">
            {t('workspacePeers.noRelayAccounts')}
          </p>
        ) : (
          <select
            value={selectedRelayAccountId ?? ''}
            onChange={e => onRelayAccountSelect?.(e.target.value)}
            disabled={disabled}
            className="text-xs px-1.5 py-0.5 rounded border border-[var(--color-border)] bg-[var(--color-background)] text-[var(--color-foreground)] mt-0.5"
          >
            <option value="" disabled>{t('workspacePeers.selectRelay')}</option>
            {relayAccounts.map(acct => (
              <option key={acct.relayAccountId} value={acct.relayAccountId}>
                {acct.email} @ {acct.relayUrl}
              </option>
            ))}
          </select>
        )
      )}

      {currentFolderPath && selectedType === 'folder' && (
        <span className="text-xs text-[var(--color-muted-foreground)] truncate font-mono" title={currentFolderPath}>
          {currentFolderPath}
        </span>
      )}
    </div>
  );
}

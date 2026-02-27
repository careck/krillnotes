import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { WorkspaceMetadata } from '../types';

const PREDEFINED_LICENSES = [
  'CC BY 4.0',
  'CC BY-SA 4.0',
  'CC BY-NC 4.0',
  'CC0 1.0',
  'MIT',
  'Apache 2.0',
  'All Rights Reserved',
  'Other\u2026',
] as const;

const OTHER_LICENSE = 'Other\u2026';

const LICENSE_URLS: Partial<Record<string, string>> = {
  'CC BY 4.0':          'https://creativecommons.org/licenses/by/4.0/',
  'CC BY-SA 4.0':       'https://creativecommons.org/licenses/by-sa/4.0/',
  'CC BY-NC 4.0':       'https://creativecommons.org/licenses/by-nc/4.0/',
  'CC0 1.0':            'https://creativecommons.org/publicdomain/zero/1.0/',
  'MIT':                'https://opensource.org/licenses/MIT',
  'Apache 2.0':         'https://www.apache.org/licenses/LICENSE-2.0',
  'All Rights Reserved': '',
};

interface WorkspacePropertiesDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function WorkspacePropertiesDialog({ isOpen, onClose }: WorkspacePropertiesDialogProps) {
  const [authorName, setAuthorName] = useState('');
  const [authorOrg, setAuthorOrg] = useState('');
  const [homepageUrl, setHomepageUrl] = useState('');
  const [description, setDescription] = useState('');
  const [licenseSelect, setLicenseSelect] = useState('');
  const [licenseCustom, setLicenseCustom] = useState('');
  const [licenseUrl, setLicenseUrl] = useState('');
  const [language, setLanguage] = useState('');
  const [tagsRaw, setTagsRaw] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    invoke<WorkspaceMetadata>('get_workspace_metadata')
      .then(meta => {
        setAuthorName(meta.authorName ?? '');
        setAuthorOrg(meta.authorOrg ?? '');
        setHomepageUrl(meta.homepageUrl ?? '');
        setDescription(meta.description ?? '');
        setLicenseUrl(meta.licenseUrl ?? '');
        setLanguage(meta.language ?? '');
        setTagsRaw(meta.tags.join(', '));
        setError('');

        const lic = meta.license ?? '';
        if (PREDEFINED_LICENSES.includes(lic as typeof PREDEFINED_LICENSES[number]) && lic !== OTHER_LICENSE) {
          setLicenseSelect(lic);
          setLicenseCustom('');
        } else if (lic !== '') {
          setLicenseSelect(OTHER_LICENSE);
          setLicenseCustom(lic);
        } else {
          setLicenseSelect('');
          setLicenseCustom('');
        }
      })
      .catch(err => setError(`Failed to load workspace properties: ${err}`));
  }, [isOpen]);

  // Auto-fill license URL when a predefined license is selected.
  useEffect(() => {
    if (licenseSelect && licenseSelect !== OTHER_LICENSE) {
      setLicenseUrl(LICENSE_URLS[licenseSelect] ?? '');
    }
  }, [licenseSelect]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const resolvedLicense = licenseSelect === OTHER_LICENSE ? licenseCustom : licenseSelect;

  const parseTags = (raw: string): string[] =>
    raw.split(',').map(t => t.trim()).filter(t => t.length > 0);

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      const metadata: WorkspaceMetadata = {
        version: 1,
        authorName: authorName || undefined,
        authorOrg: authorOrg || undefined,
        homepageUrl: homepageUrl || undefined,
        description: description || undefined,
        license: resolvedLicense || undefined,
        licenseUrl: licenseUrl || undefined,
        language: language || undefined,
        tags: parseTags(tagsRaw),
      };
      await invoke('set_workspace_metadata', { metadata });
      onClose();
    } catch (err) {
      setError(`Failed to save workspace properties: ${err}`);
    } finally {
      setSaving(false);
    }
  };

  const field = (label: string, children: React.ReactNode) => (
    <div className="mb-3">
      <label className="block text-sm font-medium mb-1">{label}</label>
      {children}
    </div>
  );

  const inputClass = 'w-full bg-secondary border border-secondary rounded px-3 py-1.5 text-sm';

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-[520px] max-h-[85vh] overflow-y-auto">
        <h2 className="text-xl font-bold mb-4">Workspace Properties</h2>
        <p className="text-sm text-muted-foreground mb-4">
          This information is embedded in the exported <code>.krillnotes</code> archive and can be
          used by template galleries to display information about this workspace.
        </p>

        {field('Author Name', (
          <input type="text" value={authorName} onChange={e => setAuthorName(e.target.value)}
            className={inputClass} placeholder="e.g. Jane Smith"
            autoCorrect="off" autoCapitalize="off" spellCheck={false} />
        ))}

        {field('Author Organisation', (
          <input type="text" value={authorOrg} onChange={e => setAuthorOrg(e.target.value)}
            className={inputClass} placeholder="e.g. ACME Corp"
            autoCorrect="off" autoCapitalize="off" spellCheck={false} />
        ))}

        {field('Homepage URL', (
          <input type="text" value={homepageUrl} onChange={e => setHomepageUrl(e.target.value)}
            className={inputClass} placeholder="https://example.com"
            autoCorrect="off" autoCapitalize="off" spellCheck={false} />
        ))}

        {field('Description', (
          <textarea value={description} onChange={e => setDescription(e.target.value)}
            className={`${inputClass} resize-y min-h-[80px]`}
            placeholder="A short description of this workspace template…"
            spellCheck={false} />
        ))}

        {field('Language', (
          <input type="text" value={language} onChange={e => setLanguage(e.target.value)}
            className={inputClass} placeholder="e.g. en"
            autoCorrect="off" autoCapitalize="off" spellCheck={false} />
        ))}

        {field('License', (
          <div className="flex flex-col gap-1.5">
            <select value={licenseSelect} onChange={e => setLicenseSelect(e.target.value)}
              className={`${inputClass} bg-background`}>
              <option value="">— select a license —</option>
              {PREDEFINED_LICENSES.map(l => (
                <option key={l} value={l}>{l}</option>
              ))}
            </select>
            {licenseSelect === OTHER_LICENSE && (
              <input type="text" value={licenseCustom} onChange={e => setLicenseCustom(e.target.value)}
                className={inputClass} placeholder="Enter license name…"
                autoCorrect="off" autoCapitalize="off" spellCheck={false} />
            )}
          </div>
        ))}

        {field('License URL', (() => {
          const isPredefined = licenseSelect !== '' && licenseSelect !== OTHER_LICENSE;
          return (
            <input type="text" value={licenseUrl}
              onChange={e => setLicenseUrl(e.target.value)}
              readOnly={isPredefined}
              className={`${inputClass} ${isPredefined ? 'opacity-50 cursor-default' : ''}`}
              placeholder="https://creativecommons.org/licenses/by/4.0/"
              autoCorrect="off" autoCapitalize="off" spellCheck={false} />
          );
        })())}

        {field('Workspace Tags', (
          <>
            <input type="text" value={tagsRaw} onChange={e => setTagsRaw(e.target.value)}
              className={inputClass} placeholder="e.g. productivity, zettelkasten, notes"
              autoCorrect="off" autoCapitalize="off" spellCheck={false} />
            <p className="text-xs text-muted-foreground mt-1">
              Comma-separated tags for gallery discovery. These are separate from per-note tags.
            </p>
          </>
        ))}

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2 mt-4">
          <button onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={saving}>
            Cancel
          </button>
          <button onClick={handleSave}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={saving}>
            {saving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  );
}

export default WorkspacePropertiesDialog;

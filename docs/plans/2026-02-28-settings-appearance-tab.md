# Settings Appearance Tab + Sync i18n Fix — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix sync settings not translating (bug), and move the Appearance section to its own Settings tab.

**Architecture:** Two independent changes — (1) correct translation strings in 6 locale JSON files, (2) restructure `SettingsDialog.tsx` to add a third tab and relocate the Appearance JSX block there. No Rust changes, no new Tauri commands.

**Tech Stack:** React + react-i18next, TypeScript, JSON locale files, Tauri

---

## Task 1: Create git worktree

**Step 1: Create the worktree and branch**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/settings-appearance-tab -b feat/settings-appearance-tab
```

Expected: `.worktrees/feat/settings-appearance-tab/` directory created.

All remaining work happens inside this worktree:
```
/Users/careck/Source/Krillnotes/.worktrees/feat/settings-appearance-tab/
```

---

## Task 2: Fix sync translation keys in all 6 non-English locale files

**Root cause:** PR #50 added four keys to `en.json` but copy-pasted English text into all other locales.

**Files to modify:**
- `krillnotes-desktop/src/i18n/locales/de.json`
- `krillnotes-desktop/src/i18n/locales/fr.json`
- `krillnotes-desktop/src/i18n/locales/es.json`
- `krillnotes-desktop/src/i18n/locales/ja.json`
- `krillnotes-desktop/src/i18n/locales/ko.json`
- `krillnotes-desktop/src/i18n/locales/zh.json`

**Step 1: Fix `de.json` — replace the four buggy entries**

In the `settings` block, replace:
```json
    "tabGeneral": "General",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "Sync keeps your notes up to date across devices. Coming soon."
```
with:
```json
    "tabGeneral": "Allgemein",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "Sync hält deine Notizen auf allen Geräten auf dem neuesten Stand. Kommt bald."
```

**Step 2: Fix `fr.json`**

Replace:
```json
    "tabGeneral": "General",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "Sync keeps your notes up to date across devices. Coming soon."
```
with:
```json
    "tabGeneral": "Général",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "La synchronisation maintient vos notes à jour sur tous vos appareils. Bientôt disponible."
```

**Step 3: Fix `es.json`**

Replace:
```json
    "tabGeneral": "General",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "Sync keeps your notes up to date across devices. Coming soon."
```
with:
```json
    "tabGeneral": "General",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "La sincronización mantiene tus notas actualizadas en todos tus dispositivos. Próximamente."
```

Note: "General" is the same word in Spanish — that entry is coincidentally correct.

**Step 4: Fix `ja.json`**

Replace:
```json
    "tabGeneral": "General",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "Sync keeps your notes up to date across devices. Coming soon."
```
with:
```json
    "tabGeneral": "一般",
    "tabSync": "同期",
    "sync": "同期",
    "syncHint": "同期により、デバイス間でノートを常に最新の状態に保ちます。近日公開。"
```

**Step 5: Fix `ko.json`**

Replace:
```json
    "tabGeneral": "General",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "Sync keeps your notes up to date across devices. Coming soon."
```
with:
```json
    "tabGeneral": "일반",
    "tabSync": "동기화",
    "sync": "동기화",
    "syncHint": "동기화는 기기 간에 노트를 최신 상태로 유지합니다. 곧 출시됩니다."
```

**Step 6: Fix `zh.json`**

Replace:
```json
    "tabGeneral": "General",
    "tabSync": "Sync",
    "sync": "Sync",
    "syncHint": "Sync keeps your notes up to date across devices. Coming soon."
```
with:
```json
    "tabGeneral": "通用",
    "tabSync": "同步",
    "sync": "同步",
    "syncHint": "同步可让您的笔记在各设备间保持最新状态。即将推出。"
```

**Step 7: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/de.json \
        krillnotes-desktop/src/i18n/locales/fr.json \
        krillnotes-desktop/src/i18n/locales/es.json \
        krillnotes-desktop/src/i18n/locales/ja.json \
        krillnotes-desktop/src/i18n/locales/ko.json \
        krillnotes-desktop/src/i18n/locales/zh.json
git commit -m "fix: translate sync settings keys in all non-English locales

PR #50 added tabGeneral, tabSync, sync, syncHint to en.json but
copy-pasted English text into all other locale files. Now properly
translated in de, fr, es, ja, ko, zh.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 3: Add `tabAppearance` translation key to all 7 locale files

**Files to modify:**
- `krillnotes-desktop/src/i18n/locales/en.json`
- `krillnotes-desktop/src/i18n/locales/de.json`
- `krillnotes-desktop/src/i18n/locales/fr.json`
- `krillnotes-desktop/src/i18n/locales/es.json`
- `krillnotes-desktop/src/i18n/locales/ja.json`
- `krillnotes-desktop/src/i18n/locales/ko.json`
- `krillnotes-desktop/src/i18n/locales/zh.json`

**Step 1: Add to `en.json`**

In the `settings` block, after `"tabSync": "Sync"`, add:
```json
    "tabAppearance": "Appearance",
```

**Step 2: Add to `de.json`** — after `"tabSync": "Sync"`:
```json
    "tabAppearance": "Erscheinungsbild",
```
Note: `"appearance": "Erscheinungsbild"` already exists in de.json — use the same value.

**Step 3: Add to `fr.json`** — after `"tabSync": "Sync"`:
```json
    "tabAppearance": "Apparence",
```

**Step 4: Add to `es.json`** — after `"tabSync": "Sync"`:
```json
    "tabAppearance": "Apariencia",
```

**Step 5: Add to `ja.json`** — after `"tabSync": "同期"`:
```json
    "tabAppearance": "外観",
```

**Step 6: Add to `ko.json`** — after `"tabSync": "동기화"`:
```json
    "tabAppearance": "외관",
```

**Step 7: Add to `zh.json`** — after `"tabSync": "同步"`:
```json
    "tabAppearance": "外观",
```

**Step 8: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/en.json \
        krillnotes-desktop/src/i18n/locales/de.json \
        krillnotes-desktop/src/i18n/locales/fr.json \
        krillnotes-desktop/src/i18n/locales/es.json \
        krillnotes-desktop/src/i18n/locales/ja.json \
        krillnotes-desktop/src/i18n/locales/ko.json \
        krillnotes-desktop/src/i18n/locales/zh.json
git commit -m "feat: add tabAppearance translation key to all locale files

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 4: Restructure SettingsDialog.tsx — add Appearance tab

**File:** `krillnotes-desktop/src/components/SettingsDialog.tsx`

**Step 1: Extend the `activeTab` state type**

Find line 25:
```tsx
  const [activeTab, setActiveTab] = useState<'general' | 'sync'>('general');
```
Replace with:
```tsx
  const [activeTab, setActiveTab] = useState<'general' | 'appearance' | 'sync'>('general');
```

**Step 2: Add the Appearance tab button in the tab bar**

Find the tab bar section (the `<div className="flex border-b border-border mb-4">` block, lines 102–123). It currently has two `<button>` elements. Add a third between them:

Replace the entire tab bar div:
```tsx
        {/* Tab bar */}
        <div className="flex border-b border-border mb-4">
          <button
            onClick={() => setActiveTab('general')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'general'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabGeneral')}
          </button>
          <button
            onClick={() => setActiveTab('sync')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'sync'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabSync')}
          </button>
        </div>
```
with:
```tsx
        {/* Tab bar */}
        <div className="flex border-b border-border mb-4">
          <button
            onClick={() => setActiveTab('general')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'general'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabGeneral')}
          </button>
          <button
            onClick={() => setActiveTab('appearance')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'appearance'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabAppearance')}
          </button>
          <button
            onClick={() => setActiveTab('sync')}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'sync'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {t('settings.tabSync')}
          </button>
        </div>
```

**Step 3: Remove the Appearance section from the General tab**

Inside `{activeTab === 'general' && (<>…</>)}`, find and remove the entire Appearance block — from the comment through the closing `</div>` (currently lines 170–248):

```tsx
            {/* Appearance */}
            <div className="border-t border-border pt-4 mt-4">
              <h3 className="text-sm font-semibold text-foreground mb-3">{t('settings.appearance')}</h3>

              {/* Language picker */}
              … (everything down to and including the Manage Themes button)

            </div>
```

Delete this entire block. The General tab should now end after the Cache Passwords `<div className="mb-4">` block.

**Step 4: Add the Appearance tab content block**

After the closing `}` of the `{activeTab === 'general' && (…)}` block and before the `{activeTab === 'sync' && (…)}` block, insert:

```tsx
        {activeTab === 'appearance' && (
          <div className="py-2">
            {/* Language picker */}
            <div className="flex items-center gap-2 mb-3">
              <span className="text-sm text-muted-foreground w-24">{t('settings.language')}</span>
              <select
                value={language}
                onChange={e => handleLanguageChange(e.target.value)}
                className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
              >
                <option value="en">English</option>
                <option value="de">Deutsch (de)</option>
                <option value="fr">Français (fr)</option>
                <option value="es">Español (es)</option>
                <option value="ja">日本語 (ja)</option>
                <option value="ko">한국어 (ko)</option>
                <option value="zh">中文 (zh)</option>
              </select>
            </div>

            {/* Mode toggle */}
            <div className="flex items-center gap-2 mb-3">
              <span className="text-sm text-muted-foreground w-24">{t('settings.mode')}</span>
              <div className="flex rounded border border-border overflow-hidden">
                {(['light', 'dark', 'system'] as const).map(m => (
                  <button
                    key={m}
                    onClick={() => setMode(m)}
                    className={`px-3 py-1 text-sm ${
                      activeMode === m
                        ? 'bg-primary text-primary-foreground'
                        : 'text-muted-foreground hover:text-foreground hover:bg-secondary'
                    }`}
                  >
                    {t(`settings.mode${m.charAt(0).toUpperCase() + m.slice(1)}`)}
                  </button>
                ))}
              </div>
            </div>

            {/* Light theme picker */}
            <div className="flex items-center gap-2 mb-2">
              <span className="text-sm text-muted-foreground w-24">{t('settings.lightTheme')}</span>
              <select
                value={lightThemeName}
                onChange={e => setLightTheme(e.target.value)}
                className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
              >
                <option value="light">{t('settings.lightBuiltIn')}</option>
                {themes.filter(theme => theme.hasLight).map(theme => (
                  <option key={theme.filename} value={theme.name}>{theme.name}</option>
                ))}
              </select>
            </div>

            {/* Dark theme picker */}
            <div className="flex items-center gap-2 mb-3">
              <span className="text-sm text-muted-foreground w-24">{t('settings.darkTheme')}</span>
              <select
                value={darkThemeName}
                onChange={e => setDarkTheme(e.target.value)}
                className="text-sm border border-border rounded px-2 py-1 bg-background text-foreground"
              >
                <option value="dark">{t('settings.darkBuiltIn')}</option>
                {themes.filter(theme => theme.hasDark).map(theme => (
                  <option key={theme.filename} value={theme.name}>{theme.name}</option>
                ))}
              </select>
            </div>

            <button
              onClick={() => setManageThemesOpen(true)}
              className="text-sm text-muted-foreground hover:text-foreground underline"
            >
              {t('settings.manageThemes')}
            </button>
          </div>
        )}
```

**Step 5: Run TypeScript build to verify no type errors**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors. If any errors appear, they will be related to the `activeTab` type — check that the union type in Step 1 was updated correctly.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/SettingsDialog.tsx
git commit -m "feat: move Appearance settings to its own tab

Settings dialog now has three tabs: General | Appearance | Sync.
The General tab retains only workspace directory and cache passwords.
The Appearance tab holds language, mode, and theme pickers.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 5: Final verification checklist

Before opening a PR, manually verify:

- [ ] Open Settings → switch to German → tab labels read "Allgemein / Erscheinungsbild / Sync"
- [ ] Switch to Japanese → tab labels read "一般 / 外観 / 同期"
- [ ] Sync tab content ("同期により…") is now in the target language
- [ ] Appearance tab shows Language, Mode, Light theme, Dark theme, Manage Themes — all translated
- [ ] General tab shows only Workspace Directory and Cache Passwords
- [ ] Changing language on the Appearance tab live-previews correctly
- [ ] Cancel reverts language; Save persists it
- [ ] `npx tsc --noEmit` passes with zero errors

import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';

const [settingsDialog, settings, main, buildRenderer, indexTemplate] = await Promise.all([
    readFile(new URL('../ui/src/layout/SettingsDialog.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/lib/terminal-settings.ts', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/main.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../build-renderer.cjs', import.meta.url), 'utf8'),
    readFile(new URL('../views/index.ejs', import.meta.url), 'utf8'),
]);

test('settings navigation provides a rendered icon for every section', () => {
    assert.match(settingsDialog, /data-settings-tab="font"[\s\S]*<Type/);
    assert.match(settingsDialog, /data-settings-tab="groups"[\s\S]*<Folder/);
});

test('settings uses controlled shadcn controls and React state', () => {
    assert.match(settingsDialog, /useState/);
    assert.match(settingsDialog, /<Select/);
    assert.match(settingsDialog, /<Input/);
    assert.match(settingsDialog, /<Card/);
    assert.doesNotMatch(settingsDialog, /<select\b/);
});

test('settings preserves persistence, all-terminal application, and async font chunking', () => {
    assert.match(settings, /localStorage\.getItem\(STORAGE_KEY\)/);
    assert.match(settings, /localStorage\.setItem\(STORAGE_KEY/);
    assert.match(settings, /applyToAllTerminals/);
    assert.match(settings, /FONT_CHUNK_SIZE = 12/);
    assert.match(settings, /await yieldToBrowser\(\)/);
    assert.match(settings, /warmSystemFontPresets/);
});

test('settings legacy stylesheet and partial are removed from all loaders', async () => {
    await assert.rejects(access(new URL('../assets/css/settings-dialog.css', import.meta.url)));
    await assert.rejects(access(new URL('../views/partials/settings-dialog.ejs', import.meta.url)));
    assert.doesNotMatch(main, /settings-dialog\.css/);
    assert.doesNotMatch(buildRenderer, /settings-dialog\.css/);
    assert.doesNotMatch(indexTemplate, /settings-dialog\.css/);
});

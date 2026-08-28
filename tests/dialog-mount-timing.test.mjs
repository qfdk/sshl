import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [dialog, uiManager, settingsDialog] = await Promise.all([
    readFile(new URL('../ui/src/components/ui/dialog.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/ui-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/layout/SettingsDialog.tsx', import.meta.url), 'utf8'),
]);

test('DialogContent keeps the non-portal mount required by the app shell', () => {
    assert.doesNotMatch(dialog, /<DialogPortal/);
});

test('button bindings do not gate on the dialog element existing', () => {
    assert.doesNotMatch(uiManager, /if\s*\(newConnectionBtn\s*&&\s*connectionDialog\)/);
    assert.doesNotMatch(uiManager, /if\s*\(!openBtn\s*\|\|\s*!dialog\s*\|\|\s*!form\)/);
});

test('settings is controlled by React state rather than a vanilla initializer', () => {
    assert.match(settingsDialog, /useSyncExternalStore/);
    assert.match(settingsDialog, /onOpenChange=/);
    assert.doesNotMatch(settingsDialog, /initSettingsUI/);
});

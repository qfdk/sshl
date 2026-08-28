import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';

const [dialog, manager, uiManager, appState, main, buildRenderer, indexTemplate] = await Promise.all([
    readFile(new URL('../ui/src/layout/ConnectionDialog.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/connection-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/ui-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/app-state.mjs', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/main.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../build-renderer.cjs', import.meta.url), 'utf8'),
    readFile(new URL('../views/index.ejs', import.meta.url), 'utf8'),
]);

test('connection dialog uses controlled shadcn form controls', () => {
    assert.match(dialog, /useState/);
    assert.match(dialog, /from ['"]\.\.\/components\/ui\/input['"]/);
    assert.match(dialog, /from ['"]\.\.\/components\/ui\/select['"]/);
    assert.match(dialog, /from ['"]\.\.\/components\/ui\/checkbox['"]/);
    assert.match(dialog, /<Input/);
    assert.match(dialog, /<Select/);
    assert.match(dialog, /<Checkbox/);
    assert.doesNotMatch(dialog, /<select\b/);
    assert.doesNotMatch(dialog, /<input\s+[^>]*type=["']checkbox["']/);
    assert.match(dialog, /onSubmit=\{handleSubmit\}/);
    assert.match(dialog, /editingConnection/);
});

test('connection manager accepts details instead of reading connection form DOM', () => {
    assert.match(manager, /async submitConnection\(details\)/);
    assert.match(manager, /async saveEditedConnection\(id, details\)/);
    assert.doesNotMatch(manager, /handleConnectionFormSubmit/);
    assert.doesNotMatch(manager, /handleEditConnection/);
    assert.doesNotMatch(manager, /getElementById\(['"](?:conn-|auth-type|connection-form)/);
    assert.match(manager, /setEditingConnection\(connection\)/);
});

test('editing connection state is exposed through the app state store', () => {
    assert.match(appState, /editingConnection/);
    assert.match(appState, /export function getEditingConnection\(\)/);
    assert.match(appState, /export function setEditingConnection\(connection\)/);
    assert.match(appState, /subscribers\.editingConnection/);
});

test('legacy connection dialog stylesheet is removed from all loaders', async () => {
    await assert.rejects(access(new URL('../assets/css/connection-dialog.css', import.meta.url)));
    assert.doesNotMatch(main, /connection-dialog\.css/);
    assert.doesNotMatch(buildRenderer, /connection-dialog\.css/);
    assert.doesNotMatch(indexTemplate, /connection-dialog\.css/);
});

test('ui manager no longer owns connection dialog form behavior', () => {
    assert.doesNotMatch(uiManager, /toggleAuthFields/);
    assert.doesNotMatch(uiManager, /connection-form/);
    assert.doesNotMatch(uiManager, /auth-type/);
    assert.doesNotMatch(uiManager, /browse-private-key/);
});

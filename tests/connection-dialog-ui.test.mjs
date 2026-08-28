import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';

const [dialog, manager, appState, main, index] = await Promise.all([
  readFile(new URL('../ui/src/layout/ConnectionDialog.tsx', import.meta.url), 'utf8'),
  readFile(new URL('../ui/src/features/connections/useConnections.ts', import.meta.url), 'utf8'),
  readFile(new URL('../ui/src/lib/app-state.ts', import.meta.url), 'utf8'),
  readFile(new URL('../ui/src/main.tsx', import.meta.url), 'utf8'),
  readFile(new URL('../ui/index.html', import.meta.url), 'utf8'),
]);
test('connection dialog uses controlled shadcn form controls', () => { assert.match(dialog, /useState/); assert.match(dialog, /components\/ui\/input/); assert.match(dialog, /components\/ui\/select/); assert.match(dialog, /components\/ui\/checkbox/); assert.match(dialog, /<Input/); assert.match(dialog, /<Select/); assert.match(dialog, /<Checkbox/); assert.doesNotMatch(dialog, /<select\b/); assert.doesNotMatch(dialog, /type=["']checkbox/); assert.match(dialog, /onSubmit=\{handleSubmit\}/); assert.match(dialog, /editingConnection/); });
test('connection business logic accepts details instead of reading form DOM', () => { assert.match(manager, /async|useCallback/); assert.match(manager, /submitConnection/); assert.match(manager, /saveEditedConnection/); assert.doesNotMatch(manager, /getElementById\(/); });
test('editing connection state is exposed through the app state store', () => { assert.match(appState, /editingConnection/); assert.match(appState, /export function getEditingConnection/); assert.match(appState, /export function setEditingConnection/); assert.match(appState, /subscribers\.editingConnection/); });
test('old renderer loaders are gone', async () => { await assert.rejects(access(new URL('../assets', import.meta.url))); await assert.rejects(access(new URL('../views', import.meta.url))); assert.doesNotMatch(main, /assets\/css|ipc-bridge\.js/); assert.doesNotMatch(index, /ipc-bridge|icons\.js/); });
test('dialog submit calls the React connection manager', () => { assert.match(dialog, /manager\.saveEditedConnection/); assert.match(dialog, /manager\.submitConnection/); });

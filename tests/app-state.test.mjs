import test, { beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../ui/src/lib/app-state.ts', import.meta.url), 'utf8');
const state = await import(`data:text/javascript,${encodeURIComponent(source)}`);

beforeEach(() => state.resetState());

test('starts with the default active tab and no current session', () => {
  assert.equal(state.getActiveTabId(), 'terminal');
  assert.equal(state.getCurrentSessionId(), null);
  assert.equal(state.getConnectionDialogOpen(), false);
  assert.equal(state.getSettingsDialogOpen(), false);
});
test('editing connection starts empty and can be set and cleared', () => { const connection = { id: 'connection-1', name: 'Server' }; assert.equal(state.getEditingConnection(), null); state.setEditingConnection(connection); assert.equal(state.getEditingConnection(), connection); state.setEditingConnection(null); assert.equal(state.getEditingConnection(), null); });
test('setters update the values returned by getters', () => { state.setActiveTabId('file-manager'); state.setCurrentSessionId('session-1'); assert.equal(state.getActiveTabId(), 'file-manager'); assert.equal(state.getCurrentSessionId(), 'session-1'); });
test('notifies subscribers with new and old values when a value changes', () => { const changes = []; state.subscribe('currentSessionId', (next, previous) => changes.push([next, previous])); state.setCurrentSessionId('session-1'); state.setCurrentSessionId(null); assert.deepEqual(changes, [['session-1', null], [null, 'session-1']]); });
test('does not notify subscribers when a value does not change', () => { let calls = 0; state.subscribe('currentSessionId', () => calls++); state.setCurrentSessionId(null); assert.equal(calls, 0); });
test('unsubscribe prevents future notifications', () => { let calls = 0; const unsubscribe = state.subscribe('activeTabId', () => calls++); unsubscribe(); state.setActiveTabId('file-manager'); assert.equal(calls, 0); });
test('one failing subscriber does not block other subscribers', () => { let calls = 0; state.subscribe('currentSessionId', () => { throw new Error('subscriber failed'); }); state.subscribe('currentSessionId', () => calls++); assert.doesNotThrow(() => state.setCurrentSessionId('session-1')); assert.equal(calls, 1); });
test('normalizes non-null current session IDs to strings', () => { state.setCurrentSessionId(123); assert.equal(state.getCurrentSessionId(), '123'); });
test('dialog setters update values and notify their subscribers', () => { const changes = []; state.subscribe('connectionDialogOpen', (next, previous) => changes.push(['connection', next, previous])); state.subscribe('settingsDialogOpen', (next, previous) => changes.push(['settings', next, previous])); state.setConnectionDialogOpen(true); state.setSettingsDialogOpen(true); assert.deepEqual(changes, [['connection', true, false], ['settings', true, false]]); });
test('dialog setters do not notify when the value does not change', () => { let calls = 0; state.subscribe('connectionDialogOpen', () => calls++); state.subscribe('settingsDialogOpen', () => calls++); state.setConnectionDialogOpen(false); state.setSettingsDialogOpen(false); assert.equal(calls, 0); });

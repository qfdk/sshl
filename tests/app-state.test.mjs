import test, { beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import {
    getActiveTabId,
    getConnectionDialogOpen,
    getSettingsDialogOpen,
    getCurrentSessionId,
    getEditingConnection,
    resetState,
    setActiveTabId,
    setConnectionDialogOpen,
    setSettingsDialogOpen,
    setCurrentSessionId,
    setEditingConnection,
    subscribe,
} from '../assets/js/app-state.mjs';

beforeEach(() => {
    resetState();
});

test('starts with the default active tab and no current session', () => {
    assert.equal(getActiveTabId(), 'terminal');
    assert.equal(getCurrentSessionId(), null);
    assert.equal(getConnectionDialogOpen(), false);
    assert.equal(getSettingsDialogOpen(), false);
});

test('editing connection starts empty and can be set and cleared', () => {
    const connection = { id: 'connection-1', name: 'Server' };

    assert.equal(getEditingConnection(), null);
    setEditingConnection(connection);
    assert.equal(getEditingConnection(), connection);
    setEditingConnection(null);
    assert.equal(getEditingConnection(), null);
});

test('setters update the values returned by getters', () => {
    setActiveTabId('file-manager');
    setCurrentSessionId('session-1');

    assert.equal(getActiveTabId(), 'file-manager');
    assert.equal(getCurrentSessionId(), 'session-1');
});

test('notifies subscribers with new and old values when a value changes', () => {
    const changes = [];
    subscribe('currentSessionId', (newValue, oldValue) => {
        changes.push([newValue, oldValue]);
    });

    setCurrentSessionId('session-1');
    setCurrentSessionId(null);

    assert.deepEqual(changes, [
        ['session-1', null],
        [null, 'session-1'],
    ]);
});

test('does not notify subscribers when a value does not change', () => {
    let calls = 0;
    subscribe('currentSessionId', () => {
        calls++;
    });

    setCurrentSessionId(null);
    assert.equal(calls, 0);
});

test('unsubscribe prevents future notifications', () => {
    let calls = 0;
    const unsubscribe = subscribe('activeTabId', () => {
        calls++;
    });

    unsubscribe();
    setActiveTabId('file-manager');

    assert.equal(calls, 0);
});

test('one failing subscriber does not block other subscribers', () => {
    let calls = 0;
    subscribe('currentSessionId', () => {
        throw new Error('subscriber failed');
    });
    subscribe('currentSessionId', () => {
        calls++;
    });

    assert.doesNotThrow(() => setCurrentSessionId('session-1'));
    assert.equal(calls, 1);
});

test('normalizes non-null current session IDs to strings', () => {
    setCurrentSessionId(123);

    assert.equal(getCurrentSessionId(), '123');
});


test('dialog setters update values and notify their subscribers', () => {
    const changes = [];
    subscribe('connectionDialogOpen', (newValue, oldValue) => {
        changes.push(['connection', newValue, oldValue]);
    });
    subscribe('settingsDialogOpen', (newValue, oldValue) => {
        changes.push(['settings', newValue, oldValue]);
    });

    setConnectionDialogOpen(true);
    setSettingsDialogOpen(true);

    assert.equal(getConnectionDialogOpen(), true);
    assert.equal(getSettingsDialogOpen(), true);
    assert.deepEqual(changes, [
        ['connection', true, false],
        ['settings', true, false],
    ]);
});

test('dialog setters do not notify when the value does not change', () => {
    let calls = 0;
    subscribe('connectionDialogOpen', () => {
        calls++;
    });
    subscribe('settingsDialogOpen', () => {
        calls++;
    });

    setConnectionDialogOpen(false);
    setSettingsDialogOpen(false);

    assert.equal(calls, 0);
});

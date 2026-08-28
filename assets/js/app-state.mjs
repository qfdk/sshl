const initialState = {
    currentSessionId: null,
    activeTabId: 'terminal',
    connectionDialogOpen: false,
    settingsDialogOpen: false,
};

let currentSessionId = initialState.currentSessionId;
let activeTabId = initialState.activeTabId;
let connectionDialogOpen = initialState.connectionDialogOpen;
let settingsDialogOpen = initialState.settingsDialogOpen;

const subscribers = {
    currentSessionId: new Set(),
    activeTabId: new Set(),
    connectionDialogOpen: new Set(),
    settingsDialogOpen: new Set(),
};

function notify(key, newValue, oldValue) {
    for (const callback of [...subscribers[key]]) {
        try {
            callback(newValue, oldValue);
        } catch {
            // A subscriber must not prevent other subscribers from running.
        }
    }
}

export function getCurrentSessionId() {
    return currentSessionId;
}

export function setCurrentSessionId(id) {
    const newValue = id == null ? null : String(id);
    if (newValue === currentSessionId) return;

    const oldValue = currentSessionId;
    currentSessionId = newValue;
    notify('currentSessionId', newValue, oldValue);
}

export function getActiveTabId() {
    return activeTabId;
}

export function setActiveTabId(id) {
    if (id === activeTabId) return;

    const oldValue = activeTabId;
    activeTabId = id;
    notify('activeTabId', id, oldValue);
}

export function getConnectionDialogOpen() {
    return connectionDialogOpen;
}

export function setConnectionDialogOpen(open) {
    const newValue = Boolean(open);
    if (newValue === connectionDialogOpen) return;

    const oldValue = connectionDialogOpen;
    connectionDialogOpen = newValue;
    notify('connectionDialogOpen', newValue, oldValue);
}

export function getSettingsDialogOpen() {
    return settingsDialogOpen;
}

export function setSettingsDialogOpen(open) {
    const newValue = Boolean(open);
    if (newValue === settingsDialogOpen) return;

    const oldValue = settingsDialogOpen;
    settingsDialogOpen = newValue;
    notify('settingsDialogOpen', newValue, oldValue);
}

export function subscribe(key, callback) {
    subscribers[key].add(callback);
    return () => subscribers[key].delete(callback);
}

export function resetState() {
    currentSessionId = initialState.currentSessionId;
    activeTabId = initialState.activeTabId;
    connectionDialogOpen = initialState.connectionDialogOpen;
    settingsDialogOpen = initialState.settingsDialogOpen;
    subscribers.currentSessionId.clear();
    subscribers.activeTabId.clear();
    subscribers.connectionDialogOpen.clear();
    subscribers.settingsDialogOpen.clear();
}

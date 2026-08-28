const initialState = {
    currentSessionId: null,
    activeTabId: 'terminal',
};

let currentSessionId = initialState.currentSessionId;
let activeTabId = initialState.activeTabId;

const subscribers = {
    currentSessionId: new Set(),
    activeTabId: new Set(),
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

export function subscribe(key, callback) {
    subscribers[key].add(callback);
    return () => subscribers[key].delete(callback);
}

export function resetState() {
    currentSessionId = initialState.currentSessionId;
    activeTabId = initialState.activeTabId;
    subscribers.currentSessionId.clear();
    subscribers.activeTabId.clear();
}

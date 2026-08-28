export const UNGROUPED_LABEL = '未分组';

export function normalizeGroupName(group) {
    return typeof group === 'string' ? group.trim() : '';
}

export function getUngroupedConnections(connections) {
    return connections.filter(connection => !normalizeGroupName(connection.group));
}

export function groupConnections(connections, groupOrder = []) {
    const groups = new Map();

    for (const connection of connections) {
        const name = normalizeGroupName(connection.group);
        if (!name) continue;
        if (!groups.has(name)) groups.set(name, []);
        groups.get(name).push(connection);
    }

    const orderedNames = [];
    const seen = new Set();
    for (const value of groupOrder) {
        const name = normalizeGroupName(value);
        if (!name || seen.has(name)) continue;
        seen.add(name);
        orderedNames.push(name);
    }
    for (const name of groups.keys()) {
        if (seen.has(name)) continue;
        seen.add(name);
        orderedNames.push(name);
    }

    return orderedNames.map(name => ({ name, connections: groups.get(name) || [] }));
}

export function moveConnection(connections, groupOrder, connectionId, targetGroup, beforeId = null) {
    const source = connections.find(connection => connection.id === connectionId);
    if (!source) return [...connections];

    const remaining = connections.filter(connection => connection.id !== connectionId);
    const updated = { ...source, group: normalizeGroupName(targetGroup) };
    const buckets = new Map();
    const defaultConnections = [];
    for (const connection of remaining) {
        const group = normalizeGroupName(connection.group);
        if (!group) {
            defaultConnections.push(connection);
            continue;
        }
        if (!buckets.has(group)) buckets.set(group, []);
        buckets.get(group).push(connection);
    }

    const targetConnections = updated.group ? (buckets.get(updated.group) || []) : defaultConnections;
    const insertAt = beforeId ? targetConnections.findIndex(connection => connection.id === beforeId) : -1;
    if (insertAt >= 0) targetConnections.splice(insertAt, 0, updated);
    else targetConnections.push(updated);

    const ordered = [...defaultConnections];
    for (const group of groupConnections(remaining, groupOrder)) {
        ordered.push(...(buckets.get(group.name) || []));
    }
    const knownGroups = new Set(groupOrder.map(normalizeGroupName));
    for (const [name, items] of buckets) {
        if (!knownGroups.has(name)) ordered.push(...items);
    }
    if (!updated.group) {
        ordered.splice(0, defaultConnections.length, ...targetConnections);
    }
    return ordered;
}

export function reorderGroups(groups, draggedName, targetName, placeAfter = false) {
    const fromIndex = groups.indexOf(draggedName);
    const targetIndex = groups.indexOf(targetName);
    if (fromIndex < 0 || targetIndex < 0 || fromIndex === targetIndex) return [...groups];

    const reordered = [...groups];
    const [dragged] = reordered.splice(fromIndex, 1);
    const adjustedTarget = targetIndex > fromIndex ? targetIndex - 1 : targetIndex;
    reordered.splice(adjustedTarget + (placeAfter ? 1 : 0), 0, dragged);
    return reordered;
}

export function matchesConnection(connection, query) {
    const needle = String(query || '').trim().toLocaleLowerCase();
    if (!needle) return true;

    return [connection.name, connection.host, connection.username, connection.group]
        .some(value => String(value || '').toLocaleLowerCase().includes(needle));
}

export function reorderConnections(connections, draggedId, targetId) {
    const fromIndex = connections.findIndex(connection => connection.id === draggedId);
    const targetIndex = connections.findIndex(connection => connection.id === targetId);
    if (fromIndex < 0 || targetIndex < 0 || fromIndex === targetIndex) return [...connections];

    const reordered = [...connections];
    const [dragged] = reordered.splice(fromIndex, 1);
    const insertIndex = targetIndex > fromIndex ? targetIndex - 1 : targetIndex;
    reordered.splice(insertIndex, 0, dragged);
    return reordered;
}

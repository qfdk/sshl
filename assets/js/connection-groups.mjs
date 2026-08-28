export const UNGROUPED_LABEL = '未分组';

export function normalizeGroupName(group) {
    const name = typeof group === 'string' ? group.trim() : '';
    return name || UNGROUPED_LABEL;
}

export function groupConnections(connections) {
    const groups = new Map();

    for (const connection of connections) {
        const name = normalizeGroupName(connection.group);
        if (!groups.has(name)) groups.set(name, []);
        groups.get(name).push(connection);
    }

    const grouped = [...groups].map(([name, items]) => ({ name, connections: items }));
    const ungroupedIndex = grouped.findIndex(group => group.name === UNGROUPED_LABEL);
    if (ungroupedIndex >= 0 && ungroupedIndex !== grouped.length - 1) {
        grouped.push(grouped.splice(ungroupedIndex, 1)[0]);
    }
    return grouped;
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

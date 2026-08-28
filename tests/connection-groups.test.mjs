import test from 'node:test';
import assert from 'node:assert/strict';
import {
    UNGROUPED_LABEL,
    groupConnections,
    matchesConnection,
    reorderConnections,
} from '../assets/js/connection-groups.mjs';

test('groups connections by trimmed group name and keeps ungrouped last', () => {
    const connections = [
        { id: '1', name: 'prod-api', group: '  Production ' },
        { id: '2', name: 'local', group: '' },
        { id: '3', name: 'prod-db', group: 'Production' },
        { id: '4', name: 'staging', group: ' Staging ' },
    ];

    assert.deepEqual(groupConnections(connections), [
        { name: 'Production', connections: [connections[0], connections[2]] },
        { name: 'Staging', connections: [connections[3]] },
        { name: UNGROUPED_LABEL, connections: [connections[1]] },
    ]);
});

test('search matches connection fields and group names', () => {
    const connection = {
        name: 'API server',
        host: '10.0.0.8',
        username: 'deploy',
        group: 'Production',
    };

    assert.equal(matchesConnection(connection, 'prod'), true);
    assert.equal(matchesConnection(connection, '10.0'), true);
    assert.equal(matchesConnection(connection, 'database'), false);
    assert.equal(matchesConnection(connection, ''), true);
});

test('reorders connections without dropping entries', () => {
    const connections = [
        { id: '1', name: 'one' },
        { id: '2', name: 'two' },
        { id: '3', name: 'three' },
    ];

    assert.deepEqual(
        reorderConnections(connections, '3', '1').map(connection => connection.id),
        ['3', '1', '2'],
    );
    assert.deepEqual(
        reorderConnections(connections, '1', '3').map(connection => connection.id),
        ['2', '1', '3'],
    );
    assert.deepEqual(
        reorderConnections(connections, 'missing', '1').map(connection => connection.id),
        ['1', '2', '3'],
    );
});

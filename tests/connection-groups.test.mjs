import test from 'node:test';
import assert from 'node:assert/strict';
import {
    getUngroupedConnections,
    groupConnections,
    moveConnection,
    matchesConnection,
    reorderConnections,
    reorderGroups,
} from '../assets/js/connection-groups.mjs';

test('orders named groups explicitly and leaves ungrouped connections at the top level', () => {
    const connections = [
        { id: '1', name: 'prod-api', group: '  Production ' },
        { id: '2', name: 'local', group: '' },
        { id: '3', name: 'prod-db', group: 'Production' },
        { id: '4', name: 'staging', group: ' Staging ' },
        { id: '5', name: 'preview', group: 'Preview' },
    ];

    assert.deepEqual(groupConnections(connections, ['Staging', 'Production', 'Empty']), [
        { name: 'Staging', connections: [connections[3]] },
        { name: 'Production', connections: [connections[0], connections[2]] },
        { name: 'Empty', connections: [] },
        { name: 'Preview', connections: [connections[4]] },
    ]);
    assert.deepEqual(getUngroupedConnections(connections), [connections[1]]);
});

test('moves a connection between groups and preserves the target position', () => {
    const connections = [
        { id: 'a', name: 'default', group: '' },
        { id: 'b', name: 'one', group: 'Production' },
        { id: 'c', name: 'two', group: 'Production' },
        { id: 'd', name: 'three', group: 'Staging' },
    ];

    assert.deepEqual(
        moveConnection(connections, ['Production', 'Staging'], 'a', 'Staging', 'd').map(connection => [connection.id, connection.group]),
        [['b', 'Production'], ['c', 'Production'], ['a', 'Staging'], ['d', 'Staging']],
    );
});

test('reorders groups without dropping names', () => {
    const groups = ['Production', 'Staging', 'Preview'];

    assert.deepEqual(reorderGroups(groups, 'Preview', 'Production'), [
        'Preview', 'Production', 'Staging',
    ]);
    assert.deepEqual(reorderGroups(groups, 'Production', 'Staging', true), [
        'Staging', 'Production', 'Preview',
    ]);
    assert.deepEqual(reorderGroups(groups, 'Missing', 'Staging'), groups);
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

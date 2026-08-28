import test from 'node:test';
import assert from 'node:assert/strict';
// package.json 是 "type": "module"，Node 直接剥离类型标注加载 .ts，无需转译步骤。
import * as groups from '../ui/src/lib/connection-groups.ts';

const connection = (id, group = '') => ({ id, name: id, host: `${id}.example`, username: 'root', group });

test('orders named groups explicitly and leaves ungrouped connections at the top level', () => { const result = groups.groupConnections([connection('a', 'prod'), connection('b'), connection('c', 'dev')], ['dev', 'prod']); assert.deepEqual(result.map(item => item.name), ['dev', 'prod']); assert.deepEqual(groups.getUngroupedConnections([connection('a', 'prod'), connection('b')]).map(item => item.id), ['b']); });
test('moves a connection between groups and preserves the target position', () => { const result = groups.moveConnection([connection('a'), connection('b', 'prod'), connection('c', 'prod')], ['prod'], 'a', 'prod', 'c'); assert.deepEqual(result.map(item => item.id), ['b', 'a', 'c']); assert.equal(result[1].group, 'prod'); });
test('reorders groups without dropping names', () => { assert.deepEqual(groups.reorderGroups(['a', 'b', 'c'], 'a', 'c'), ['b', 'a', 'c']); assert.deepEqual(groups.reorderGroups(['a', 'b', 'c'], 'c', 'a', true), ['a', 'c', 'b']); });
test('search matches connection fields and group names', () => { const item = connection('server', 'production'); assert.equal(groups.matchesConnection(item, 'PROD'), true); assert.equal(groups.matchesConnection(item, 'missing'), false); });
test('reorders connections without dropping entries', () => { assert.deepEqual(groups.reorderConnections([connection('a'), connection('b'), connection('c')], 'c', 'a').map(item => item.id), ['c', 'a', 'b']); });
test('builds collapsed-rail initials from the first grapheme of the name', () => { assert.equal(groups.getConnectionInitials({ id: '1', name: 'prod-web-01' }), 'P'); assert.equal(groups.getConnectionInitials({ id: '2', name: '生产数据库' }), '生'); assert.equal(groups.getConnectionInitials({ id: '3', name: '🚀 deploy' }), '🚀'); assert.equal(groups.getConnectionInitials({ id: '4', name: '👨‍💻 dev' }), '👨‍💻'); assert.equal(groups.getConnectionInitials({ id: '5', host: 'db.example.com' }), 'D'); assert.equal(groups.getConnectionInitials({ id: '6' }), '?'); });
test('moves a connection into an empty group without dropping it', () => { const result = groups.moveConnection([connection('a'), connection('b')], ['fresh'], 'a', 'fresh', null); assert.deepEqual(result.map(item => item.id).sort(), ['a', 'b']); assert.equal(result.find(item => item.id === 'a').group, 'fresh'); });
test('moves a connection into an empty group that is not in the group order', () => { const result = groups.moveConnection([connection('a')], [], 'a', 'fresh', null); assert.deepEqual(result.map(item => item.id), ['a']); assert.equal(result[0].group, 'fresh'); });

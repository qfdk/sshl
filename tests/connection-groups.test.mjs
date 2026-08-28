import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

let source = await readFile(new URL('../ui/src/lib/connection-groups.ts', import.meta.url), 'utf8');
const groups = await import(`data:text/javascript,${encodeURIComponent(source)}`);
const connection = (id, group = '') => ({ id, name: id, host: `${id}.example`, username: 'root', group });

test('orders named groups explicitly and leaves ungrouped connections at the top level', () => { const result = groups.groupConnections([connection('a', 'prod'), connection('b'), connection('c', 'dev')], ['dev', 'prod']); assert.deepEqual(result.map(item => item.name), ['dev', 'prod']); assert.deepEqual(groups.getUngroupedConnections([connection('a', 'prod'), connection('b')]).map(item => item.id), ['b']); });
test('moves a connection between groups and preserves the target position', () => { const result = groups.moveConnection([connection('a'), connection('b', 'prod'), connection('c', 'prod')], ['prod'], 'a', 'prod', 'c'); assert.deepEqual(result.map(item => item.id), ['b', 'a', 'c']); assert.equal(result[1].group, 'prod'); });
test('reorders groups without dropping names', () => { assert.deepEqual(groups.reorderGroups(['a', 'b', 'c'], 'a', 'c'), ['b', 'a', 'c']); assert.deepEqual(groups.reorderGroups(['a', 'b', 'c'], 'c', 'a', true), ['a', 'c', 'b']); });
test('search matches connection fields and group names', () => { const item = connection('server', 'production'); assert.equal(groups.matchesConnection(item, 'PROD'), true); assert.equal(groups.matchesConnection(item, 'missing'), false); });
test('reorders connections without dropping entries', () => { assert.deepEqual(groups.reorderConnections([connection('a'), connection('b'), connection('c')], 'c', 'a').map(item => item.id), ['c', 'a', 'b']); });

import test from 'node:test';
import assert from 'node:assert/strict';
import { findConnection } from '../ui/src/features/connections/useConnections.ts';

const inMemory = [
  { id: 'c1', name: 'web-01', host: '10.0.1.11' },
  { id: 'c2', name: 'db-01', host: '10.0.1.12' },
];

function loader(rows) {
  const calls = { count: 0 };
  return { calls, load: async () => { calls.count += 1; return rows; } };
}

test('内存列表里有：直接用，不再走 IPC 读库', async () => {
  const { calls, load } = loader([]);
  assert.deepEqual(await findConnection(inMemory, 'c2', load), inMemory[1]);
  assert.equal(calls.count, 0);
});

test('内存里还没有（列表尚未刷新）：回落读库', async () => {
  const fresh = { id: 'c3', name: 'new-box', host: '10.0.1.13' };
  const { calls, load } = loader([...inMemory, fresh]);
  assert.deepEqual(await findConnection(inMemory, 'c3', load), fresh);
  assert.equal(calls.count, 1);
});

test('哪里都没有：null', async () => {
  const { load } = loader(inMemory);
  assert.equal(await findConnection([], 'gone', load), null);
});

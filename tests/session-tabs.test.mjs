import test from 'node:test';
import assert from 'node:assert/strict';
import { toSessionTabs } from '../ui/src/lib/session-tabs.ts';

const connections = [
  { id: 'c1', name: 'web-01', host: '10.0.1.11' },
  { id: 'c4', name: 'dev-box', host: '192.168.1.40' },
];

test('one tab per open session, in the order the sessions were opened', () => {
  const sessions = [
    { id: 's2', connectionId: 'c4', name: 'dev-box' },
    { id: 's5', connectionId: 'c1', name: 'web-01' },
  ];
  const tabs = toSessionTabs(sessions, connections, 's5');
  assert.deepEqual(tabs.map(tab => tab.sessionId), ['s2', 's5']);
  assert.deepEqual(tabs.map(tab => tab.active), [false, true]);
  assert.equal(tabs[1].host, '10.0.1.11');
  assert.equal(tabs[1].connectionId, 'c1');
});

test('takes the current name from the saved connection, falling back to the session snapshot', () => {
  const sessions = [
    { id: 's1', connectionId: 'c1', name: 'old-name' },
    { id: 's9', connectionId: 'gone', name: 'temp-host' },
  ];
  const tabs = toSessionTabs(sessions, connections, null);
  assert.equal(tabs[0].name, 'web-01');
  assert.equal(tabs[1].name, 'temp-host');
  assert.equal(tabs[1].host, '');
  assert.ok(tabs.every(tab => !tab.active));
});

test('no sessions means no tabs', () => {
  assert.deepEqual(toSessionTabs([], connections, null), []);
});

const tab = id => ({ sessionId: id });

test('dragged order wins; tabs opened since keep their opening order at the end', async () => {
  const { applyTabOrder } = await import('../ui/src/lib/session-tabs.ts');
  const tabs = ['s1', 's2', 's3', 's4'].map(tab);
  // s3 已被拖到最前；s4 是之后新开的；order 里的 s9 已关闭
  const ordered = applyTabOrder(tabs, ['s3', 's1', 's9', 's2']);
  assert.deepEqual(ordered.map(item => item.sessionId), ['s3', 's1', 's2', 's4']);
  assert.deepEqual(applyTabOrder(tabs, []).map(item => item.sessionId), ['s1', 's2', 's3', 's4']);
});

test('moves a dragged tab before or after the drop target', async () => {
  const { moveTabId } = await import('../ui/src/lib/session-tabs.ts');
  const ids = ['a', 'b', 'c', 'd'];
  assert.deepEqual(moveTabId(ids, 'd', 'b', 'before'), ['a', 'd', 'b', 'c']);
  assert.deepEqual(moveTabId(ids, 'a', 'c', 'after'), ['b', 'c', 'a', 'd']);
  assert.deepEqual(moveTabId(ids, 'a', 'd', 'after'), ['b', 'c', 'd', 'a']);
  assert.deepEqual(moveTabId(ids, 'b', 'b', 'before'), ids);
  assert.deepEqual(moveTabId(ids, 'x', 'b', 'before'), ids);
});

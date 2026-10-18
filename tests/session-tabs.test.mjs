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

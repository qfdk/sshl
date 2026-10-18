import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createSessionRevalidator } from '../ui/src/lib/session-revalidator.ts';

function setup({ alive = true, current = 's1' } = {}) {
  const calls = { validate: [], discard: [], reconnect: [] };
  let resolveValidate;
  const state = { current };
  const revalidator = createSessionRevalidator({
    validate: sessionId => {
      calls.validate.push(sessionId);
      return new Promise(resolve => { resolveValidate = resolve; });
    },
    getCurrentSessionId: () => state.current,
    discard: (sessionId, connectionId) => calls.discard.push([sessionId, connectionId]),
    reconnect: connectionId => calls.reconnect.push(connectionId),
  });
  const settle = (result = { success: true, alive }) => resolveValidate(result);
  return { calls, state, revalidator, settle };
}

test('a live session is left alone', async () => {
  const { calls, revalidator, settle } = setup();
  const done = revalidator.revalidate('s1', 'c1');
  settle();
  await done;
  assert.deepEqual(calls.discard, []);
  assert.deepEqual(calls.reconnect, []);
});

test('a dead session the user is still viewing is discarded and reconnected', async () => {
  const { calls, revalidator, settle } = setup({ alive: false });
  const done = revalidator.revalidate('s1', 'c1');
  settle();
  await done;
  assert.deepEqual(calls.discard, [['s1', 'c1']]);
  assert.deepEqual(calls.reconnect, ['c1']);
});

test('a failed probe IPC counts as dead', async () => {
  const { calls, revalidator, settle } = setup();
  const done = revalidator.revalidate('s1', 'c1');
  settle({ success: false, error: 'ipc down' });
  await done;
  assert.deepEqual(calls.reconnect, ['c1']);
});

test('a rejected probe counts as dead and does not leave the session stuck as pending', async () => {
  const reconnect = [];
  let probes = 0;
  const revalidator = createSessionRevalidator({
    validate: () => { probes += 1; return Promise.reject(new Error('boom')); },
    getCurrentSessionId: () => 's1',
    discard: () => {},
    reconnect: connectionId => reconnect.push(connectionId),
  });
  await revalidator.revalidate('s1', 'c1');
  assert.deepEqual(reconnect, ['c1']);
  await revalidator.revalidate('s1', 'c1');
  assert.equal(probes, 2);
});

test('ssh:closed may clear the current session before the probe answers; still reconnect', async () => {
  const { calls, state, revalidator, settle } = setup({ alive: false });
  const done = revalidator.revalidate('s1', 'c1');
  state.current = null;
  settle();
  await done;
  assert.deepEqual(calls.reconnect, ['c1']);
});

test('a dead session the user already switched away from is discarded without reconnecting', async () => {
  const { calls, state, revalidator, settle } = setup({ alive: false });
  const done = revalidator.revalidate('s1', 'c1');
  state.current = 's2';
  settle();
  await done;
  assert.deepEqual(calls.discard, [['s1', 'c1']]);
  assert.deepEqual(calls.reconnect, []);
});

test('a user disconnect during the probe wins: no discard, no reconnect', async () => {
  const { calls, state, revalidator, settle } = setup({ alive: false });
  const done = revalidator.revalidate('s1', 'c1');
  revalidator.forget('s1');
  state.current = null;
  settle();
  await done;
  assert.deepEqual(calls.discard, []);
  assert.deepEqual(calls.reconnect, []);
});

test('rapid re-clicks do not stack probes for the same session', async () => {
  const { calls, revalidator, settle } = setup();
  const done = revalidator.revalidate('s1', 'c1');
  assert.equal(revalidator.revalidate('s1', 'c1'), undefined);
  assert.deepEqual(calls.validate, ['s1']);
  settle();
  await done;
});

// 切换会话不能再 await 探活：探活要开关一个 probe 通道（≥1 RTT，最坏 2s 超时）。
test('switching sessions does not wait for the liveness probe', async () => {
  const source = await readFile(new URL('../ui/src/features/connections/useConnections.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /await\s+appWindow\(\)\.api\.ssh\.validateSession/);
  assert.match(source, /revalidator\.revalidate\(/);
  assert.match(source, /revalidator\.forget\(/);
});

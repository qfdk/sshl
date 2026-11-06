import test from 'node:test';
import assert from 'node:assert/strict';
import { mock } from 'bun:test';

// Tauri 的 listen() 是异步注册：resolve 之前拿不到取消函数。用可控的假 listen 模拟「注册还没完成就卸载」。
const registrations = [];
mock.module('@tauri-apps/api/event', () => ({
  listen: (name, handler) => new Promise(resolve => {
    const registration = { name, handler, stopped: 0 };
    registration.resolve = () => resolve(() => { registration.stopped += 1; });
    registrations.push(registration);
  }),
}));
mock.module('@tauri-apps/api/core', () => ({ invoke: async () => null }));
mock.module('@tauri-apps/plugin-dialog', () => ({ open: async () => null }));

// ipc.ts 加载时把 api 挂到 window 上
const hadWindow = 'window' in globalThis;
if (!hadWindow) globalThis.window = globalThis;
const { api } = await import('../ui/src/lib/ipc.ts');
if (!hadWindow) delete globalThis.window;

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

test('注册完成后取消：调用 Tauri 的取消函数', async () => {
  const seen = [];
  const stop = api.file.onUploadProgress((_event, payload) => seen.push(payload));
  const registration = registrations.at(-1);
  assert.equal(registration.name, 'file:upload-progress');
  registration.resolve();
  await flush();
  registration.handler({ payload: { transferred: 1 } });
  assert.deepEqual(seen, [{ transferred: 1 }]);
  stop();
  assert.equal(registration.stopped, 1);
});

test('注册还没完成就取消（组件先卸载 / StrictMode 双挂载）：注册完成后立刻取消，不留下永久监听', async () => {
  const seen = [];
  const stop = api.file.onDownloadProgress((_event, payload) => seen.push(payload));
  const registration = registrations.at(-1);
  stop();
  // 取消之后、注册完成之前到达的事件也不再回调
  registration.handler({ payload: { transferred: 2 } });
  registration.resolve();
  await flush();
  assert.equal(registration.stopped, 1);
  assert.deepEqual(seen, []);
});

test('ssh:closed 走同一套注册：提前取消同样不泄漏', async () => {
  const stop = api.ssh.onClosed(() => {});
  const registration = registrations.at(-1);
  assert.equal(registration.name, 'ssh:closed');
  stop();
  registration.resolve();
  await flush();
  assert.equal(registration.stopped, 1);
});

test('connections:updated 提前取消同样不泄漏', async () => {
  let calls = 0;
  const stop = api.config.onConnectionsUpdated(() => { calls += 1; });
  const registration = registrations.at(-1);
  assert.equal(registration.name, 'connections:updated');
  stop();
  registration.handler({ payload: null });
  registration.resolve();
  await flush();
  assert.equal(registration.stopped, 1);
  assert.equal(calls, 0);
});

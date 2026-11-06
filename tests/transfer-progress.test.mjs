import test from 'node:test';
import assert from 'node:assert/strict';
import { createTransferStatusStore, describeProgress } from '../ui/src/features/files/transfer-progress.ts';

test('单文件进度：百分比与字节数，不带文件计数', () => {
  assert.deepEqual(describeProgress('下载', { remotePath: '/srv/app.log', transferred: 512 * 1024, total: 1024 * 1024 }), {
    progress: 50,
    info: '正在下载: app.log (50% - 512.00 KB/1.00 MB)',
  });
});

test('目录进度：附带已完成/总文件数', () => {
  assert.deepEqual(
    describeProgress('上传', { remotePath: '/home/q/logs', transferred: 300 * 1024, total: 1200 * 1024, filesDone: 3, filesTotal: 10 }),
    { progress: 25, info: '正在上传: logs (25% - 300.00 KB/1.17 MB · 3/10 个文件)' },
  );
});

test('目录路径以 / 结尾时仍取到目录名', () => {
  assert.match(describeProgress('下载', { remotePath: '/home/q/logs/', transferred: 0, total: 10, filesDone: 0, filesTotal: 1 }).info, /^正在下载: logs \(/);
});

test('总量为 0 不出 NaN，超出总量封顶 100%', () => {
  assert.equal(describeProgress('下载', { remotePath: '/e', transferred: 0, total: 0, filesDone: 0, filesTotal: 0 }).progress, 0);
  assert.equal(describeProgress('下载', { remotePath: '/f', transferred: 20, total: 10 }).progress, 100);
});

test('进度条状态 store：初始空闲；整体替换与按当前值更新都通知订阅者，取消订阅后不再通知', () => {
  const store = createTransferStatusStore();
  assert.deepEqual(store.get(), { active: false, progress: 0, info: '' });
  const seen = [];
  const unsubscribe = store.subscribe(() => seen.push(store.get()));
  store.set({ active: true, progress: 40, info: '正在上传: a' });
  // 传输结束 3 秒后收起：只改 active / progress，保留最后一条文字
  store.set(current => ({ ...current, active: false, progress: 0 }));
  unsubscribe();
  store.set({ active: true, progress: 1, info: 'x' });
  assert.deepEqual(seen, [
    { active: true, progress: 40, info: '正在上传: a' },
    { active: false, progress: 0, info: '正在上传: a' },
  ]);
  assert.equal(store.get().info, 'x');
});

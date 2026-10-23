import test from 'node:test';
import assert from 'node:assert/strict';
import { alertDialog, confirmDialog, currentDialog, restoreFocusAfterDialogs, settleDialog, subscribeDialogs } from '../ui/src/lib/dialogs.ts';
import { disconnectPrompt } from '../ui/src/features/connections/disconnect-prompt.ts';
import { deletePrompt } from '../ui/src/features/files/delete-prompt.ts';

test('确认框：确定 → true，取消/Esc → false；中文按钮', async () => {
  const yes = confirmDialog({ title: '断开连接', message: 'x', confirmLabel: '断开' });
  assert.equal(currentDialog().confirmLabel, '断开');
  assert.equal(currentDialog().cancelLabel, '取消');
  settleDialog(true);
  assert.equal(await yes, true);

  const no = confirmDialog({ title: '删除连接' });
  assert.equal(currentDialog().confirmLabel, '确定');
  settleDialog(false);
  assert.equal(await no, false);
  assert.equal(currentDialog(), null);
});

test('多个请求排队，一个一个显示，按顺序结算', async () => {
  let notified = 0;
  const stop = subscribeDialogs(() => { notified += 1; });
  const first = alertDialog({ title: '删除失败', message: 'Permission denied' });
  const second = confirmDialog({ title: '断开连接' });
  assert.equal(currentDialog().title, '删除失败');
  assert.equal(currentDialog().kind, 'alert');
  assert.equal(currentDialog().confirmLabel, '知道了');
  settleDialog(false);
  assert.equal(await first, undefined);
  assert.equal(currentDialog().title, '断开连接');
  settleDialog(true);
  assert.equal(await second, true);
  assert.equal(currentDialog(), null);
  assert.ok(notified >= 4, '入队与结算都要通知界面');
  stop();
});

test('断开连接：所有入口同一个标题与按钮，后果说明单独一行', () => {
  assert.deepEqual(disconnectPrompt('tiny', { transferring: false, onFileTab: false }), {
    title: '断开连接', message: '确定要断开「tiny」吗？', confirmLabel: '断开',
  });
  assert.equal(disconnectPrompt('tiny', { transferring: false, onFileTab: true }).message, '确定要断开「tiny」吗？\n文件管理会一并关闭。');
  // 传输中优先提示传输会中断
  assert.equal(disconnectPrompt('tiny', { transferring: true, onFileTab: true }).message, '确定要断开「tiny」吗？\n正在进行的文件传输会中断。');
});

test('删除确认：单个文件/目录点名；多个只列前几个，不把整份清单塞进对话框', () => {
  assert.deepEqual(deletePrompt([{ path: '/a/b.txt', isDirectory: false }]), {
    title: '删除文件', message: '确定要删除「b.txt」吗？\n此操作不可恢复。', confirmLabel: '删除',
  });
  assert.equal(deletePrompt([{ path: '/a/logs', isDirectory: true }]).message, '确定要删除「logs」及其所有内容吗？\n此操作不可恢复。');
  const many = Array.from({ length: 188 }, (_, i) => ({ path: `/a/f${i}`, isDirectory: false }));
  const prompt = deletePrompt(many);
  assert.equal(prompt.title, '删除 188 个项目');
  // 文件名清单走 detail（单行、超出省略），不在名字中间折行；数量已在标题里
  assert.equal(prompt.detail, 'f0、f1、f2、f3、f4、f5、f6、f7、f8、f9');
  assert.equal(prompt.message, '此操作不可恢复。');
  assert.equal(deletePrompt(many.slice(0, 3)).detail, 'f0、f1、f2');
});

test('detail 透传到对话框请求', () => {
  void confirmDialog({ title: 't', message: 'm', detail: 'a、b' });
  assert.equal(currentDialog().detail, 'a、b');
  settleDialog(false);
});

test('最后一个对话框关闭后，焦点还给弹出前的元素（如终端）；排队中途不动', async () => {
  let focused = 0;
  const terminalInput = { isConnected: true, focus: () => { focused += 1; } };
  globalThis.document = { activeElement: terminalInput };
  try {
    const first = alertDialog({ title: '上传失败' });
    const second = alertDialog({ title: '下载失败' });
    settleDialog(true);
    await first;
    restoreFocusAfterDialogs();
    assert.equal(focused, 0, '还有排队的对话框，不还焦点');
    settleDialog(true);
    await second;
    restoreFocusAfterDialogs();
    assert.equal(focused, 1);
    restoreFocusAfterDialogs();
    assert.equal(focused, 1, '只还一次');
  } finally {
    delete globalThis.document;
  }
});

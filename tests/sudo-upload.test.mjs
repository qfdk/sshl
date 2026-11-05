import test from 'node:test';
import assert from 'node:assert/strict';
import { currentDialog, passwordDialog, settleDialog } from '../ui/src/lib/dialogs.ts';
import { isPermissionDenied, sudoFailure, uploadWithSudoFallback } from '../ui/src/features/files/sudo-upload.ts';

const denied = { success: false, error: 'SFTP 错误: open remote /home/bob/a.txt: Permission denied: Permission denied' };

test('识别 SFTP 权限不足，其他错误不算', () => {
  assert.equal(isPermissionDenied(denied.error), true);
  assert.equal(isPermissionDenied('SFTP 错误: open remote /x: No such file: No such file'), false);
  assert.equal(isPermissionDenied(undefined), false);
});

test('后端 sudo 失败标记 → 前端分类', () => {
  assert.equal(sudoFailure('SUDO_PASSWORD_REQUIRED: sudo: a password is required'), 'password-required');
  assert.equal(sudoFailure('SUDO_PASSWORD_INCORRECT: Sorry, try again.'), 'password-incorrect');
  assert.equal(sudoFailure('SUDO_NOT_ALLOWED: alice is not in the sudoers file.'), 'not-allowed');
  assert.equal(sudoFailure('SUDO_FAILED: tar: boom'), 'other');
});

function harness({ normal, sudoResults, confirm = true, passwords = [] }) {
  const calls = { sudo: [], confirm: 0, ask: [] };
  const run = () => uploadWithSudoFallback({
    normal: async () => normal,
    sudo: async password => { calls.sudo.push(password); return sudoResults.shift(); },
    confirm: async () => { calls.confirm += 1; return confirm; },
    askPassword: async retry => { calls.ask.push(retry); return passwords.length ? passwords.shift() : null; },
  });
  return { run, calls };
}

test('普通上传成功：不问、不走 sudo', async () => {
  const { run, calls } = harness({ normal: { success: true }, sudoResults: [] });
  assert.deepEqual(await run(), { success: true });
  assert.equal(calls.confirm, 0);
  assert.deepEqual(calls.sudo, []);
});

test('非权限错误原样返回，不提 sudo', async () => {
  const err = { success: false, error: 'SFTP 错误: open remote /x: No such file' };
  const { run, calls } = harness({ normal: err, sudoResults: [] });
  assert.deepEqual(await run(), err);
  assert.equal(calls.confirm, 0);
});

test('权限不足 → 用户拒绝 sudo → 标记取消（不弹报错）', async () => {
  const { run, calls } = harness({ normal: denied, sudoResults: [], confirm: false });
  assert.deepEqual(await run(), { success: false, cancelled: true });
  assert.deepEqual(calls.sudo, []);
});

test('权限不足 → 确认 → 先用保存的密码/免密 sudo；需要密码再弹框重试', async () => {
  const { run, calls } = harness({
    normal: denied,
    sudoResults: [{ success: false, error: 'SUDO_PASSWORD_REQUIRED: a password is required' }, { success: true }],
    passwords: ['pw1'],
  });
  assert.deepEqual(await run(), { success: true });
  assert.deepEqual(calls.sudo, [null, 'pw1']);
  assert.deepEqual(calls.ask, [false]);
});

test('密码错误再问，最多三次后返回最后的错误', async () => {
  const wrong = { success: false, error: 'SUDO_PASSWORD_INCORRECT: Sorry, try again.' };
  const { run, calls } = harness({ normal: denied, sudoResults: [wrong, wrong, wrong, wrong], passwords: ['a', 'b', 'c'] });
  assert.deepEqual(await run(), wrong);
  assert.deepEqual(calls.sudo, [null, 'a', 'b', 'c']);
  assert.deepEqual(calls.ask, [true, true, true]);
});

test('要密码时用户取消 → 标记取消', async () => {
  const { run } = harness({ normal: denied, sudoResults: [{ success: false, error: 'SUDO_PASSWORD_REQUIRED: x' }], passwords: [] });
  assert.deepEqual(await run(), { success: false, cancelled: true });
});

test('没有 sudo 权限：不再要密码，直接返回错误', async () => {
  const err = { success: false, error: 'SUDO_NOT_ALLOWED: alice is not in the sudoers file.' };
  const { run, calls } = harness({ normal: denied, sudoResults: [err] });
  assert.deepEqual(await run(), err);
  assert.deepEqual(calls.ask, []);
});

test('密码框：确定返回输入值，取消返回 null', async () => {
  const yes = passwordDialog({ title: '需要 sudo 密码' });
  assert.equal(currentDialog().kind, 'prompt');
  assert.equal(currentDialog().inputType, 'password');
  settleDialog(true, 'secret');
  assert.equal(await yes, 'secret');

  const no = passwordDialog({ title: '需要 sudo 密码' });
  settleDialog(false, 'ignored');
  assert.equal(await no, null);
  assert.equal(currentDialog(), null);
});

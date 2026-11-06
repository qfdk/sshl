import test from 'node:test';
import assert from 'node:assert/strict';
import { getFileIdentity, setFileIdentity, subscribeFileIdentity, syncFileUser } from '../ui/src/features/files/file-user.ts';
import sessionManager from '../ui/src/lib/session-manager.ts';

const wrong = { success: false, error: 'SUDO_PASSWORD_INCORRECT: su: Authentication failure' };

function harness({ who, switchResults = [], passwords = [] }) {
  const calls = { switch: [], ask: [] };
  const run = (options = {}) => syncFileUser({
    identify: async () => who,
    switchUser: async (target, secret) => { calls.switch.push([target, secret]); return switchResults.shift() ?? { success: true }; },
    askPassword: async (retry, exclude, context) => { calls.ask.push({ retry, exclude: [...exclude], ...context }); return passwords.length ? passwords.shift() : null; },
    ...options,
  });
  return { run, calls };
}

const alice = { loginUser: 'alice', user: 'alice', via: null, credential: 'password', fileUser: 'alice' };

test('终端和文件管理是同一个用户：什么都不做', async () => {
  const { run, calls } = harness({ who: alice });
  assert.deepEqual(await run(), { status: 'same', fileUser: 'alice', terminalUser: 'alice' });
  assert.deepEqual(calls.switch, []);
});

test('终端 su 成 root 且保存了 root 的密码：直接用它切过去，不弹框', async () => {
  const { run, calls } = harness({ who: { ...alice, user: 'root', via: 'su', credential: 'acct:root' } });
  assert.deepEqual(await run(), { status: 'switched', fileUser: 'root', terminalUser: 'root' });
  assert.deepEqual(calls.switch, [[{ user: 'root', via: 'su' }, { credential: 'acct:root' }]]);
  assert.deepEqual(calls.ask, []);
});

test('没保存 root 的密码：弹框（说明要谁的密码），选/输之后切换', async () => {
  const { run, calls } = harness({
    who: { ...alice, user: 'root', via: 'su', credential: null },
    switchResults: [{ success: false, error: 'SUDO_PASSWORD_REQUIRED: 需要 root 的密码' }, wrong, { success: true }],
    passwords: [{ credential: 'acct:admin' }, { password: 'typed' }],
  });
  assert.equal((await run()).status, 'switched');
  // 第一次没密码不会真跑 su（后端直接回 REQUIRED）；之后都是用户亲手选/输的
  assert.deepEqual(calls.switch.map(([, secret]) => secret), [null, { credential: 'acct:admin' }, { password: 'typed' }]);
  assert.deepEqual(calls.ask.map(a => [a.retry, a.exclude, a.user, a.via, a.needUser]), [
    [false, [], 'root', 'su', 'root'],
    [true, ['acct:admin'], 'root', 'su', 'root'],
  ]);
});

test('sudo -i 切过去的：要的是登录用户的 sudo 密码', async () => {
  const { run, calls } = harness({
    who: { ...alice, user: 'root', via: 'sudo', credential: null },
    switchResults: [{ success: false, error: 'SUDO_PASSWORD_REQUIRED: a password is required' }, { success: true }],
    passwords: [{ password: 'alicepw' }],
  });
  assert.equal((await run()).status, 'switched');
  assert.equal(calls.ask[0].needUser, 'alice');
  assert.deepEqual(calls.switch[1], [{ user: 'root', via: 'sudo' }, { password: 'alicepw' }]);
});

test('用户取消输密码：保持原身份，标记取消', async () => {
  const { run } = harness({
    who: { ...alice, user: 'root', via: 'su', credential: null },
    switchResults: [{ success: false, error: 'SUDO_PASSWORD_REQUIRED: x' }],
  });
  assert.deepEqual(await run(), { status: 'cancelled', fileUser: 'alice', terminalUser: 'root' });
});

test('自动同步时不重复打扰：取消过的用户不再弹框，点徽标（interactive）才再问', async () => {
  const who = { ...alice, user: 'root', via: 'su', credential: null };
  const quiet = harness({ who, switchResults: [{ success: false, error: 'SUDO_PASSWORD_REQUIRED: x' }] });
  assert.equal((await quiet.run({ prompt: false })).status, 'cancelled');
  assert.deepEqual(quiet.calls.switch, [], '不弹框也不白跑一次');
  const asked = harness({ who, switchResults: [{ success: false, error: 'SUDO_PASSWORD_REQUIRED: x' }], passwords: [{ password: 'rootpw' }] });
  assert.equal((await asked.run({ prompt: true })).status, 'switched');
});

test('终端 exit 回到登录用户：文件管理也切回来', async () => {
  const { run, calls } = harness({ who: { ...alice, fileUser: 'root' } });
  assert.deepEqual(await run(), { status: 'switched', fileUser: 'alice', terminalUser: 'alice' });
  assert.deepEqual(calls.switch, [[null, null]]);
});

test('没有 sudo 权限 / su 被拒：失败并带上原因', async () => {
  const { run } = harness({
    who: { ...alice, user: 'root', via: 'sudo', credential: 'password' },
    switchResults: [{ success: false, error: 'SUDO_NOT_ALLOWED: alice is not in the sudoers file.' }],
  });
  const result = await run();
  assert.equal(result.status, 'failed');
  assert.match(result.error, /SUDO_NOT_ALLOWED/);
});

test('探测失败：当作没变化', async () => {
  const { run, calls } = harness({ who: null });
  assert.deepEqual(await run(), { status: 'same', fileUser: null, terminalUser: null });
  assert.deepEqual(calls.switch, []);
});

test('每个会话记住文件管理当前身份', () => {
  setFileIdentity('s1', { loginUser: 'alice', fileUser: 'root' });
  assert.deepEqual(getFileIdentity('s1'), { loginUser: 'alice', fileUser: 'root' });
  assert.equal(getFileIdentity('s2'), null);
  setFileIdentity('s1', null);
  assert.equal(getFileIdentity('s1'), null);
});

test('会话被移除（断开 / 远端关闭 / 失效丢弃）时，它的文件管理身份一并清掉，其他会话不受影响', () => {
  sessionManager.addSession('closing', 'c1');
  sessionManager.addSession('staying', 'c2');
  setFileIdentity('closing', { loginUser: 'alice', fileUser: 'root' });
  setFileIdentity('staying', { loginUser: 'bob', fileUser: 'bob' });
  let notified = 0;
  const unsubscribe = subscribeFileIdentity(() => { notified += 1; });
  sessionManager.removeSession('closing');
  unsubscribe();
  assert.equal(getFileIdentity('closing'), null);
  assert.deepEqual(getFileIdentity('staying'), { loginUser: 'bob', fileUser: 'bob' });
  assert.ok(notified > 0, '徽标订阅者收到变化');
  sessionManager.removeSession('staying');
  assert.equal(getFileIdentity('staying'), null);
});

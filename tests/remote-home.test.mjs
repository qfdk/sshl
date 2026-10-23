import test from 'node:test';
import assert from 'node:assert/strict';
import { remoteHomeFromResult, resolveRemoteHome } from '../ui/src/features/files/useFileList.ts';
import sessionManager from '../ui/src/lib/session-manager.ts';

test('takes the home directory from the SFTP realpath of "."', () => {
  assert.equal(remoteHomeFromResult({ success: true, home: '/home/ubuntu' }), '/home/ubuntu');
  assert.equal(remoteHomeFromResult({ success: true, home: '/root' }), '/root');
});

test('a failed or unusable result gives no home, so the caller falls back to /', () => {
  assert.equal(remoteHomeFromResult({ success: false, error: 'channel closed' }), '');
  assert.equal(remoteHomeFromResult({ success: true, home: '' }), '');
  assert.equal(remoteHomeFromResult({ success: true, home: 'relative/dir' }), '');
  assert.equal(remoteHomeFromResult(null), '');
});

test('the home comes from SFTP, not from running a shell command', async () => {
  const calls = [];
  globalThis.window = {
    api: {
      file: { remoteHome: async sessionId => { calls.push(['remoteHome', sessionId]); return { success: true, home: '/home/ubuntu' }; } },
      ssh: { execute: async (sessionId, command) => { calls.push(['execute', sessionId, command]); return { success: true, output: '/home/ubuntu\n' }; } },
    },
  };
  try {
    assert.equal(await resolveRemoteHome('s1'), '/home/ubuntu');
    // exec 要多开一个通道并在远端起一次 shell；只允许 SFTP 的服务器上还会直接失败。
    assert.deepEqual(calls, [['remoteHome', 's1']]);
  } finally {
    delete globalThis.window;
  }
});

test('a new session has no remote directory yet, so the file manager can start from home', () => {
  sessionManager.addSession('s-home', 'c1');
  assert.equal(sessionManager.getRemotePath('s-home'), '');
  sessionManager.updateRemotePath('s-home', '/var/log');
  assert.equal(sessionManager.getRemotePath('s-home'), '/var/log');
  sessionManager.removeSession('s-home');
});

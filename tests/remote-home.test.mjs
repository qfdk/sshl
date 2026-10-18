import test from 'node:test';
import assert from 'node:assert/strict';
import { remoteHomeFromResult } from '../ui/src/features/files/useFileList.ts';
import sessionManager from '../ui/src/lib/session-manager.ts';

test('takes the home directory from the echo "$HOME" output', () => {
  assert.equal(remoteHomeFromResult({ success: true, output: '/home/ubuntu\n' }), '/home/ubuntu');
  assert.equal(remoteHomeFromResult({ success: true, output: '/root' }), '/root');
});

test('a failed or unusable result gives no home, so the caller falls back to /', () => {
  assert.equal(remoteHomeFromResult({ success: false, error: 'channel closed' }), '');
  assert.equal(remoteHomeFromResult({ success: true, output: '' }), '');
  assert.equal(remoteHomeFromResult({ success: true, output: 'Welcome!\n/root\n' }), '');
  assert.equal(remoteHomeFromResult(null), '');
});

test('a new session has no remote directory yet, so the file manager can start from home', () => {
  sessionManager.addSession('s-home', 'c1');
  assert.equal(sessionManager.getRemotePath('s-home'), '');
  sessionManager.updateRemotePath('s-home', '/var/log');
  assert.equal(sessionManager.getRemotePath('s-home'), '/var/log');
  sessionManager.removeSession('s-home');
});

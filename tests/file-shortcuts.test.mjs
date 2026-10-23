import test from 'node:test';
import assert from 'node:assert/strict';
import { isSelectAllShortcut } from '../ui/src/features/files/shortcuts.ts';

const key = (overrides) => ({ key: 'a', metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...overrides });

test('Cmd+A / Ctrl+A 是全选', () => {
  assert.equal(isSelectAllShortcut(key({ metaKey: true })), true);
  assert.equal(isSelectAllShortcut(key({ ctrlKey: true })), true);
  // 大写锁定时 key 是 'A'
  assert.equal(isSelectAllShortcut(key({ key: 'A', metaKey: true })), true);
});

test('单按 A、带 Shift/Option 的组合不是全选', () => {
  assert.equal(isSelectAllShortcut(key({})), false);
  assert.equal(isSelectAllShortcut(key({ metaKey: true, shiftKey: true })), false);
  assert.equal(isSelectAllShortcut(key({ metaKey: true, altKey: true })), false);
  assert.equal(isSelectAllShortcut(key({ key: 'q', metaKey: true })), false);
});

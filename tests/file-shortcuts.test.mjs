import test from 'node:test';
import assert from 'node:assert/strict';
import { isClearSelectionKey, isSelectAllShortcut, preventUiSelectAll } from '../ui/src/features/files/shortcuts.ts';

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

// 模拟事件目标：closest 只认可编辑元素选择器。
const target = (editable) => ({ closest: selector => (editable && selector.includes('input') ? {} : null) });
const keydown = (overrides, editable) => {
  const event = { ...key(overrides), target: target(editable), prevented: false };
  event.preventDefault = () => { event.prevented = true; };
  return event;
};

test('界面上（非可编辑元素）的 Cmd+A 被拦下，不整页选中文字', () => {
  const event = keydown({ metaKey: true }, false);
  preventUiSelectAll(event);
  assert.equal(event.prevented, true);
});

test('输入框、终端里的 Cmd+A 放行；其他按键不拦', () => {
  const inInput = keydown({ metaKey: true }, true);
  preventUiSelectAll(inInput);
  assert.equal(inInput.prevented, false);
  const copy = keydown({ key: 'c', metaKey: true }, false);
  preventUiSelectAll(copy);
  assert.equal(copy.prevented, false);
});

test('Esc 取消选择；带修饰键、输入法选字中的 Esc 不算', () => {
  assert.equal(isClearSelectionKey({ ...key({ key: 'Escape' }), isComposing: false }), true);
  // 输入法候选框里的 Esc 是取消输入，不能顺手把文件选择也清掉
  assert.equal(isClearSelectionKey({ ...key({ key: 'Escape' }), isComposing: true }), false);
  assert.equal(isClearSelectionKey({ ...key({ key: 'Escape', metaKey: true }), isComposing: false }), false);
  assert.equal(isClearSelectionKey({ ...key({ key: 'a' }), isComposing: false }), false);
});

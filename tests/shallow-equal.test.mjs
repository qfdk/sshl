import test from 'node:test';
import assert from 'node:assert/strict';
import { shallowEqual } from '../ui/src/lib/shallow-equal.ts';

// 会话标签、连接列表、分组树拖拽时每次移动都算出一个新的落点对象：内容没变就不 setState
test('会话标签的落点：同一个标签的同一侧算没变，换了一侧 / 换了标签算变了', () => {
  assert.equal(shallowEqual({ id: 's1', side: 'before' }, { id: 's1', side: 'before' }), true);
  assert.equal(shallowEqual({ id: 's1', side: 'before' }, { id: 's1', side: 'after' }), false);
  assert.equal(shallowEqual({ id: 's1', side: 'after' }, { id: 's2', side: 'after' }), false);
});

test('连接列表的落点：同一项同一边、同一分组、都是移出分组算没变', () => {
  const item = { kind: 'item', group: 'web', itemId: 'c1', edge: 'top', beforeId: 'c1' };
  assert.equal(shallowEqual(item, { ...item }), true);
  assert.equal(shallowEqual(item, { ...item, edge: 'bottom', beforeId: 'c2' }), false, '越过中线');
  assert.equal(shallowEqual({ kind: 'item', group: 'web', itemId: 'c3', edge: 'bottom', beforeId: null }, { kind: 'item', group: 'web', itemId: 'c3', edge: 'bottom', beforeId: null }), true);
  assert.equal(shallowEqual({ kind: 'group', group: 'web' }, { kind: 'group', group: 'web' }), true);
  assert.equal(shallowEqual({ kind: 'group', group: 'web' }, { kind: 'group', group: 'db' }), false);
  assert.equal(shallowEqual({ kind: 'ungrouped' }, { kind: 'ungrouped' }), true);
  assert.equal(shallowEqual({ kind: 'ungrouped' }, { kind: 'group', group: '' }), false, '字段不同的两种落点');
});

test('分组树的落点：排序位置 before 为 null 与具体分组不同', () => {
  assert.equal(shallowEqual({ kind: 'groupOrder', before: null, marker: 'db' }, { kind: 'groupOrder', before: null, marker: 'db' }), true);
  assert.equal(shallowEqual({ kind: 'groupOrder', before: null, marker: 'db' }, { kind: 'groupOrder', before: 'db', marker: 'db' }), false, '同一分组的上半 / 下半');
  assert.equal(shallowEqual({ kind: 'intoGroup', group: '' }, { kind: 'intoGroup', group: '' }), true);
});

test('没有落点（指针离开列表）：都为 null 算没变，一边为 null 算变了', () => {
  assert.equal(shallowEqual(null, null), true);
  assert.equal(shallowEqual(null, { kind: 'ungrouped' }), false);
  assert.equal(shallowEqual({ kind: 'ungrouped' }, null), false);
});

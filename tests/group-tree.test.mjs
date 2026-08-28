import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const tree = await readFile(new URL('../ui/src/features/connections/GroupTree.tsx', import.meta.url), 'utf8');

// 拖拽反馈必须由 React 依据 state 渲染。运行时 classList.add 拼出来的 Tailwind 类
// 没有对应 CSS——Tailwind 在编译期扫描源码，扫不到运行时才拼出的类名。
test('drag feedback comes from state, not runtime class assembly', () => {
    assert.doesNotMatch(tree, /classList\.add\(/);
    assert.match(tree, /drop\?\.kind === 'group'/);
});

// 阈值之下算点击；没有它，轻轻一碰就会误触发排序。
test('drag needs to clear a movement threshold first', () => {
    assert.match(tree, /DRAG_THRESHOLD = 6/);
    assert.match(tree, /distance < DRAG_THRESHOLD/);
});

// 收尾挂 window：指针可能在树外松开，容器级监听收不到，拖拽状态会卡住。
test('drag cleanup is bound at window level', () => {
    assert.match(tree, /window\.addEventListener\('pointerup'/);
    assert.match(tree, /window\.addEventListener\('pointercancel'/);
});

// 指针离开树时不产生落点，松手即取消，不会把机器甩到别处。
test('leaving the tree cancels instead of dropping arbitrarily', () => {
    assert.match(tree, /if \(!element \|\| !root\.contains\(element\)\) return null;/);
});

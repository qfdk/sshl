import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const tree = await readFile(new URL('../ui/src/features/connections/GroupTree.tsx', import.meta.url), 'utf8');

// 拖拽反馈必须由 React 依据 state 渲染。运行时 classList.add 拼出来的 Tailwind 类
// 没有对应 CSS——Tailwind 在编译期扫描源码，扫不到运行时才拼出的类名。
test('drag feedback comes from state, not runtime class assembly', () => {
    assert.doesNotMatch(tree, /classList\.add\(/);
    assert.match(tree, /drop\?\.kind === 'intoGroup'/);
    assert.match(tree, /drop\?\.kind === 'beside'/);
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

// 展开某个分组后，里面的机器要能互相排序（组内与跨组都走同一条路径）。
test('machines can be reordered inside and across groups', () => {
    assert.match(tree, /kind: 'beside'/);
    assert.match(tree, /beforeId: before \? row\.dataset\.treeConnection/);
    assert.match(tree, /onMoveConnection\(current\.id, target\.group, target\.beforeId\)/);
});

// 默认全部折叠：整棵树收成一列分组时最好拖动排序，展开后才排里面的机器。
test('groups start collapsed so the tree reads as a sortable list', () => {
    assert.match(tree, /useState<Set<string>>\(new Set\(\)\)/);
    assert.match(tree, /const isExpanded = expanded\.has\(group\)/);
});

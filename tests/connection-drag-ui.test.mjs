import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const [list, manager] = await Promise.all([readFile(new URL('../ui/src/features/connections/ConnectionList.tsx', import.meta.url), 'utf8'), readFile(new URL('../ui/src/features/connections/useConnections.ts', import.meta.url), 'utf8')]);
test('connection drag updates data and rerenders through the pure move function', () => { assert.match(list, /manager\.move/); assert.match(manager, /moveConnection/); assert.doesNotMatch(list, /appendChild/); });
test('connection drag has a 6px threshold and window-level cleanup', () => { assert.match(list, /distance\s*<\s*6/); assert.match(list, /window\.addEventListener\('pointerup'/); assert.match(list, /window\.addEventListener\('pointercancel'/); assert.match(list, /onPointerDown/); assert.match(list, /onPointerMove/); });

// 拖拽反馈必须由 React 依据 state 渲染。运行时 classList.add 拼出来的 Tailwind 类
// 没有对应 CSS——Tailwind 在编译期扫描源码，扫不到运行时才拼出的类名。
test('drag feedback is rendered from state, not assembled at runtime', () => {
    assert.doesNotMatch(list, /classList\.add\(/);
    assert.match(list, /dropEdge === 'top'/);
    assert.match(list, /dropEdge === 'bottom'/);
});

// 落在列表内、任何分组之外 = 移出分组。未分组区为空时高度为 0，必须在拖拽时给它
// 最小高度，否则永远拖不进去。
test('dropping outside any group moves the connection out of its group', () => {
    assert.match(list, /return \{ kind: 'ungrouped' \};/);
    assert.match(list, /dragging && 'min-h-10/);
    assert.match(list, /kind === 'ungrouped' \? '' : target\.group/);
});

// 指针离开列表时不产生落点，松手即取消。
test('leaving the list cancels rather than dropping somewhere arbitrary', () => {
    assert.match(list, /if \(!element \|\| !list\.contains\(element\)\) return null;/);
});

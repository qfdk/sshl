import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [terminalCss, mainTsx, list] = await Promise.all([
    readFile(new URL('../ui/src/features/terminal/terminal.css', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/main.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/features/connections/ConnectionList.tsx', import.meta.url), 'utf8'),
]);

// xterm 自己生成 DOM，Tailwind 和 shadcn 都够不到它，这些覆盖必须是真实 CSS。
// 丢了它们的后果：内容贴边（无内边距）、内部层不透明导致黑缝、滚动条占位在右侧
// 留一条未渲染的缝。
test('xterm overrides survive: padding, transparent layers, hidden scrollbar', () => {
    assert.match(terminalCss, /\.terminal-container \.xterm\s*\{[\s\S]*padding:\s*8px\s*!important/);
    assert.match(terminalCss, /\.xterm-rows[\s\S]*background-color:\s*transparent\s*!important/);
    assert.match(terminalCss, /\.xterm-viewport::-webkit-scrollbar/);
});

// 这些规则靠 !important 压过 xterm 自带样式，必须排在 xterm.css 之后导入。
test('terminal overrides load after xterm.css', () => {
    const xtermAt = mainTsx.indexOf("xterm/css/xterm.css");
    const overrideAt = mainTsx.indexOf("features/terminal/terminal.css");
    assert.ok(xtermAt !== -1 && overrideAt !== -1, 'both stylesheets must be imported');
    assert.ok(overrideAt > xtermAt, 'terminal.css must come after xterm.css');
});

// 收起后行内只剩状态点，名称要靠 hover 提示补回来。
// 两个 Radix Trigger 必须链式 asChild 包住同一个 DOM 元素。把 <ContextMenu> 整体
// 塞进 TooltipTrigger asChild 时，hover 事件落在 Radix Root 组件上而非真实 DOM，
// 提示永远不会弹出。
test('collapsed connection rows expose their name through a tooltip', () => {
    assert.match(list, /<TooltipTrigger asChild>\{itemDiv\}<\/TooltipTrigger>/);
    assert.match(list, /TooltipContent side="right"/);
    assert.doesNotMatch(list, /<TooltipTrigger asChild>\{row\}/);
});

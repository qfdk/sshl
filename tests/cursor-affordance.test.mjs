import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(new URL('../ui/src/index.css', import.meta.url), 'utf8');

// Tailwind v4 的 preflight 把 button 的 cursor 从 pointer 改成 default，所以每个
// shadcn 按钮都会显示成箭头而不是手形。base 层里必须把它恢复回来，禁用态除外。
test('buttons keep a pointer cursor despite the Tailwind v4 preflight change', () => {
    assert.match(css, /button:not\(:disabled\)[\s\S]{0,200}cursor:\s*pointer/);
    assert.match(css, /\[role="button"\]:not\(:disabled\)/);
});

// 折叠态的宽度必须放得下 size="icon" 的按钮（40px）加上容器内边距，
// 否则收起按钮会被挤出侧边栏。w-14(56px) 配 px-3(24px) 就装不下。
test('collapsed sidebar is wide enough for its icon buttons', async () => {
    const sidebar = await readFile(new URL('../ui/src/layout/Sidebar.tsx', import.meta.url), 'utf8');
    assert.doesNotMatch(sidebar, /collapsed \? 'w-14'/);
    assert.match(sidebar, /collapsed \? 'w-16' : 'w-72'/);
    // 宽度类必须互斥，不能同时留在 class 里靠声明顺序决胜
    assert.doesNotMatch(sidebar, /\bw-72\b[^`]*\$\{collapsed \? 'w-1/);
});

// 收起侧边栏后仍要能看到每台机器的状态点——这是原本的行为，整块卸载连接列表就没了。
test('collapsed sidebar still renders the connection list as status dots', async () => {
    const [sidebar, list] = await Promise.all([
        readFile(new URL('../ui/src/layout/Sidebar.tsx', import.meta.url), 'utf8'),
        readFile(new URL('../ui/src/features/connections/ConnectionList.tsx', import.meta.url), 'utf8'),
    ]);
    // 列表不能被包在 {!collapsed && ...} 里
    assert.doesNotMatch(sidebar, /\{!collapsed && <>[\s\S]*ConnectionList/);
    assert.match(sidebar, /<ConnectionList[^/]*collapsed=\{collapsed\}/);
    // 折叠态下名称隐藏、状态点保留
    assert.match(list, /\{!collapsed && <span className="min-w-0 flex-1 truncate"/);
    assert.match(list, /data-connection-status/);
});

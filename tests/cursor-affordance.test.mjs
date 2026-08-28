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

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [dialog, uiManager, settings] = await Promise.all([
    readFile(new URL('../ui/src/components/ui/dialog.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/ui-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/settings.js', import.meta.url), 'utf8'),
]);

// Radix 的 Portal 内部用 useState(false) + effect 延迟一帧挂载，而 vanilla manager 在
// App 的 useEffect 里同步 getElementById 找对话框内的元素。走 Portal 时这些元素全是 null：
// ui-manager 的事件绑定整块跳过、initSettingsUI 直接 return —— 表现为按钮点了没反应。
test('DialogContent renders without a portal so managers can find it during init', () => {
    assert.doesNotMatch(dialog, /<DialogPortal/);
});

// handler 早已改用 store setter，不再需要对话框元素本身；把它留在守卫条件里会让绑定
// 依赖 Portal 的挂载时机。
test('button bindings do not gate on the dialog element existing', () => {
    assert.doesNotMatch(uiManager, /if\s*\(newConnectionBtn\s*&&\s*connectionDialog\)/);
    assert.doesNotMatch(uiManager, /if\s*\(cancelConnectionBtn\s*&&\s*connectionForm\s*&&\s*connectionDialog\)/);
});

test('settings init still guards on its own elements', () => {
    assert.match(settings, /if\s*\(!openBtn\s*\|\|\s*!dialog\s*\|\|\s*!form\)\s*return;/);
});

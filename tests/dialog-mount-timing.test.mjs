import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [dialog, uiManager, settingsDialog, connectionDialog] = await Promise.all([
    readFile(new URL('../ui/src/components/ui/dialog.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/ui-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/layout/SettingsDialog.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/layout/ConnectionDialog.tsx', import.meta.url), 'utf8'),
]);

// forceMount 绕过 Radix 的 Presence，使 DialogContent 内部的 RemoveScroll 在对话框关闭时
// 仍然生效，给 document.body 挂上 pointer-events:none。后果是所有按钮的真实鼠标点击全部
// 失效，而程序化 el.click() 依旧触发 —— 极易被误判为「功能正常」。
// 表单已完全受控，没有 vanilla 代码需要对话框 DOM 常驻，因此不该再出现 forceMount。
test('dialogs never use forceMount, which would lock body pointer-events', () => {
    assert.doesNotMatch(dialog, /forceMount/);
    assert.doesNotMatch(connectionDialog, /forceMount/);
    assert.doesNotMatch(settingsDialog, /forceMount/);
});

test('button bindings do not gate on the dialog element existing', () => {
    assert.doesNotMatch(uiManager, /if\s*\(newConnectionBtn\s*&&\s*connectionDialog\)/);
    assert.doesNotMatch(uiManager, /if\s*\(!openBtn\s*\|\|\s*!dialog\s*\|\|\s*!form\)/);
});

test('settings is controlled by React state rather than a vanilla initializer', () => {
    assert.match(settingsDialog, /useSyncExternalStore/);
    assert.match(settingsDialog, /onOpenChange=/);
    assert.doesNotMatch(settingsDialog, /initSettingsUI/);
});

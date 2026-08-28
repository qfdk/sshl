import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const files = await Promise.all(
    ['../ui/src/App.tsx', '../ui/src/layout/ConnectionDialog.tsx', '../ui/src/layout/SettingsDialog.tsx'].map(
        (path) => readFile(new URL(path, import.meta.url), 'utf8'),
    ),
);

// useSyncExternalStore 的 subscribe 必须是稳定引用。写成内联箭头函数时 React 会在每次
// 渲染后重新订阅，通知会在 unsubscribe/subscribe 的间隙丢失；一旦丢失，store 已经是新值，
// setter 的「值未变化直接 return」让后续调用全部短路，UI 再也不更新——表现为按钮点了没反应。
test('useSyncExternalStore never takes an inline subscribe callback', () => {
    for (const source of files) {
        assert.doesNotMatch(
            source,
            /useSyncExternalStore\(\s*\(\s*callback\s*\)\s*=>/,
            'subscribe must be a module-level constant, not an inline arrow',
        );
    }
});

test('each subscribing component holds a module-level subscribe constant', () => {
    for (const source of files) {
        assert.match(source, /^const subscribe[A-Za-z]+ = \(callback: \(\) => void\) => subscribe\('/m);
    }
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [manager, styles] = await Promise.all([
    readFile(new URL('../assets/js/connection-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/css/main.css', import.meta.url), 'utf8'),
]);

test('connection drag updates data and rerenders instead of moving DOM nodes', () => {
    assert.match(manager, /moveConnection/);
    assert.match(manager, /renderConnectionList/);
    assert.match(manager, /targetGroup/);
    assert.doesNotMatch(manager, /beginPointerDrag/);
    assert.doesNotMatch(manager, /connection-drag-placeholder/);
    assert.doesNotMatch(manager, /document\.body\.appendChild\(drag\.item\)/);
});

test('connection drag has a pointer threshold and window-level cleanup', () => {
    assert.match(manager, /active:\s*false/);
    assert.match(manager, /distance\s*<\s*6/);
    assert.match(manager, /window\.addEventListener\('pointerup'/);
    assert.match(manager, /window\.addEventListener\('pointercancel'/);
    assert.match(styles, /\.connection-item\s*\{/);
    assert.match(styles, /\.connection-group\s*\{/);
    assert.match(styles, /\.connection-status-indicator\.online\s*\{[\s\S]*var\(--success\)/);
    assert.match(styles, /\.connection-status-indicator\.offline\s*\{[\s\S]*var\(--muted-foreground\)/);
    assert.match(styles, /\.connection-actions\s*\{/);
    assert.match(styles, /\.connection-item\.dragging\s*\{/);
    assert.match(styles, /\.connection-item\.drag-over\s*\{/);
});

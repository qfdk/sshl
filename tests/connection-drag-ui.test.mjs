import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const manager = await readFile(new URL('../assets/js/connection-manager.js', import.meta.url), 'utf8');

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
});

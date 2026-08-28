import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [template, manager, groupManager, styles, icons] = await Promise.all([
    readFile(new URL('../views/partials/sidebar.ejs', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/connection-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/group-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/css/settings-dialog.css', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/icons.js', import.meta.url), 'utf8'),
]);

test('sidebar keeps the same named groups as settings and leaves ungrouped connections at the top level', () => {
    assert.match(template, /id="connection-list"[^>]*><\/div>/);
    assert.doesNotMatch(template, /class="connection-group-header"/);
    assert.match(icons, /'chevron-down':/);
    assert.doesNotMatch(template, /connection-group-header/);
    assert.match(manager, /getUngroupedConnections/);
    assert.match(manager, /connection-default-items/);
    assert.doesNotMatch(manager, /if\s*\(!group\.connections\.length\)\s*continue/);
});

test('group manager exposes drag sorting for persisted group order', () => {
    assert.match(groupManager, /group-manager-drag-handle/);
    assert.match(groupManager, /groupDrag/);
    assert.match(groupManager, /addEventListener\('pointerdown'/);
    assert.match(groupManager, /window\.addEventListener\('pointerup'/);
    assert.doesNotMatch(groupManager, /draggable\s*=\s*true/);
    assert.match(groupManager, /reorderGroups/);
    assert.match(groupManager, /getConnectionGroups/);
    assert.match(styles, /\.group-manager-drag-handle/);
});

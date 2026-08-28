import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [template, manager, groupManager, styles, icons, app, sidebar, uiManager, mainStyles] = await Promise.all([
    readFile(new URL('../views/partials/sidebar.ejs', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/connection-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/group-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/css/settings-dialog.css', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/icons.js', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/App.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../ui/src/layout/Sidebar.tsx', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/ui-manager.js', import.meta.url), 'utf8'),
    readFile(new URL('../assets/css/main.css', import.meta.url), 'utf8'),
]);

test('sidebar keeps the same named groups as settings and leaves ungrouped connections at the top level', () => {
    assert.match(template, /id="connection-list"[^>]*><\/div>/);
    assert.doesNotMatch(template, /class="connection-group-header"/);
    assert.match(icons, /'chevron-down':/);
    assert.doesNotMatch(template, /connection-group-header/);
    assert.match(manager, /getUngroupedConnections/);
    assert.match(manager, /connection-default-items/);
    assert.doesNotMatch(manager, /if\s*\(!group\.connections\.length\)\s*continue/);
    assert.match(sidebar, /<Button[\s\S]*id="new-connection-btn"/);
    assert.match(sidebar, /<Input[\s\S]*id="connection-search"/);
    assert.match(sidebar, /<Button[\s\S]*id="toggle-sidebar"[\s\S]*variant="ghost"/);
    assert.match(sidebar, /<Button[\s\S]*id="open-settings-btn"[\s\S]*variant="ghost"/);
    assert.match(sidebar, /from ['"]lucide-react['"]/);
    assert.match(app, /<Tabs[\s\S]*value=\{activeTabId\}[\s\S]*onValueChange=\{setActiveTabId\}/);
    assert.match(app, /<TabsContent[\s\S]*value="terminal"[\s\S]*id="terminal-tab"[\s\S]*forceMount[\s\S]*data-\[state=inactive\]:hidden/);
    assert.match(app, /<TabsContent[\s\S]*value="file-manager"[\s\S]*id="file-manager-tab"[\s\S]*forceMount[\s\S]*data-\[state=inactive\]:hidden/);
    assert.match(mainStyles, /\.tab-pane\[data-state="active"\]\s*\{[\s\S]*display:\s*block/);
    assert.match(app, /useSyncExternalStore/);
    assert.match(uiManager, /setActiveTabId\(tabId\)/);
    assert.doesNotMatch(uiManager, /tabs\.forEach\(t => t\.classList\.remove\('active'\)\)/);
    assert.doesNotMatch(uiManager, /tab\.classList\.add\('active'\)/);
    assert.match(uiManager, /setTimeout\(\(\) => window\.terminalManager\.resizeTerminal\(\), 300\)/);
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

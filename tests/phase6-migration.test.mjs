import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';

const files = {
    main: '../assets/js/main-entry.js',
    connectionManager: '../assets/js/connection-manager.js',
    terminal: '../ui/src/features/terminal/useTerminal.ts',
    terminalUi: '../ui/src/layout/Terminal.tsx',
    session: '../ui/src/lib/session-manager.ts',
    settings: '../ui/src/layout/SettingsDialog.tsx',
    settingsLib: '../ui/src/lib/terminal-settings.ts',
    renderer: '../build-renderer.cjs',
    index: '../views/index.ejs',
};
const content = Object.fromEntries(await Promise.all(Object.entries(files).map(async ([key, file]) => [key, await readFile(new URL(file, import.meta.url), 'utf8')])));

test('phase 6 removes the three vanilla modules and old partials', async () => {
    for (const file of ['../assets/js/terminal-manager.js', '../assets/js/settings.js', '../assets/js/session-manager.js', '../assets/css/settings-dialog.css', '../views/partials/settings-dialog.ejs', '../views/partials/terminal.ejs']) {
        await assert.rejects(access(new URL(file, import.meta.url)));
    }
    assert.doesNotMatch(content.main, /terminal-manager|settings\.js|session-manager\.js|initSettingsUI|initGroupManager/);
});

test('terminal React hook preserves renderer ordering and the seven timing-sensitive paths', () => {
    const source = content.terminal;
    assert.match(source, /term\.open\(container[\s\S]*loadCanvasRenderer\(term\)/);
    assert.match(source, /alignScreenToBottom/);
    assert.match(source, /attachResizeHandler/);
    assert.match(source, /getAnsiPalette[\s\S]*paletteToHex[\s\S]*cellBgHex[\s\S]*sampleEdgeBg[\s\S]*parseOscColor/);
    assert.match(source, /term\.loadAddon\(new Unicode11Addon\(\)\)/);
    assert.match(source, /allowProposedApi: true/);
    assert.match(source, /terminalsRef/);
});

test('terminal UI uses a React ref mount point and the hook', () => {
    assert.match(content.terminalUi, /useRef/);
    assert.match(content.terminalUi, /ref=\{containerRef\}/);
    assert.match(content.terminalUi, /useTerminal\(containerRef\)/);
});

test('session state remains a simple module-level Map', () => {
    assert.match(content.session, /new Map/);
    assert.match(content.session, /getSessionByConnectionId/);
    assert.match(content.session, /currentRemotePath/);
    assert.doesNotMatch(content.session, /zustand|redux/);
});

// 首次连接的缓冲快照由后端作为首个 ssh:data 事件发出，与后续数据同通道保序。
// 曾经的写法是 invoke getSessionBuffer 拿快照再 term.write，那样快照会和事件通道乱序，
// 导致欢迎横幅重复渲染——这是已修过的 bug，不能退回。
// 唯一允许拉缓冲区的地方是 switchToSession 里「为已有会话新建终端实例」的回放路径。
test('connect paths activate atomically instead of pulling the buffer over invoke', () => {
    const source = content.connectionManager;
    assert.equal(
        (source.match(/原子激活：缓冲快照由后端作为首个 ssh:data 事件发出/g) || []).length,
        3,
        'all three connect paths must activate atomically',
    );
    assert.equal(
        (source.match(/getSessionBuffer/g) || []).length,
        1,
        'the buffer may only be pulled when replaying into a freshly created terminal',
    );
    assert.match(source, /terminalResult\.isNew[\s\S]{0,240}getSessionBuffer/);
});

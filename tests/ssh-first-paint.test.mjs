import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const connection = await readFile(
    new URL('../ui/src/features/connections/useConnections.ts', import.meta.url),
    'utf8',
);

// 首次连接的缓冲快照由后端作为首个 ssh:data 事件发出，与后续输出同通道保序。
// 曾经的写法是 invoke getSessionBuffer 拿到快照再 term.write，那样快照会与事件通道
// 乱序，欢迎横幅重复渲染或首屏错位——这个 bug 修过两次，第二次是某个 agent 照着一份
// 过时的文档描述把它改了回去，靠这条断言拦下的。
//
// 唯一允许拉缓冲区的地方：switchToSession 中为已有会话新建终端实例时的回放。
test('connect paths activate atomically instead of pulling the buffer over invoke', () => {
    assert.equal(
        (connection.match(/原子激活：缓冲快照由后端作为首个 ssh:data 事件发出/g) || []).length,
        3,
        'all three connect paths must activate atomically',
    );
    assert.equal(
        (connection.match(/getSessionBuffer/g) || []).length,
        1,
        'the buffer may only be pulled when replaying into a freshly created terminal',
    );
    assert.match(connection, /terminalResult\.isNew[\s\S]{0,300}getSessionBuffer/);
});

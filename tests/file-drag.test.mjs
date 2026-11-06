import test from 'node:test';
import assert from 'node:assert/strict';
import { createDragPreviewStore, dragItems, dragLabel, dropPointToCss, dropTargetFrom, planDrop, releasedMove, sameDropTarget } from '../ui/src/features/files/file-drag.ts';

// 只实现 closest / getAttribute 的假 DOM 节点
function node(attrs, parent = null) {
  return {
    getAttribute: name => attrs[name] ?? null,
    closest(selector) {
      const name = selector.slice(1, -1);
      for (let n = this; n; n = n.parent) if (n.attrs[name] !== undefined) return n;
      return null;
    },
    attrs,
    parent,
  };
}
const remotePane = node({ 'data-file-pane': 'remote' });
const localPane = node({ 'data-file-pane': 'local' });
const dirs = { local: '/Users/me/Downloads', remote: '/home/bob/uploads' };

test('落点：哪一侧、是否落在文件夹行上', () => {
  assert.deepEqual(dropTargetFrom(node({}, remotePane)), { pane: 'remote', directory: null });
  assert.deepEqual(dropTargetFrom(node({}, node({ 'data-file-dir': '/home/bob/uploads/existing' }, remotePane))), { pane: 'remote', directory: '/home/bob/uploads/existing' });
  assert.equal(dropTargetFrom(node({})), null, '落在两侧之外');
  assert.equal(dropTargetFrom(null), null);
});

test('本地拖到远程 = 上传到远程当前目录 / 落点文件夹；远程拖到本地 = 下载', () => {
  assert.deepEqual(planDrop('local', { pane: 'remote', directory: null }, dirs), { kind: 'upload', directory: '/home/bob/uploads' });
  assert.deepEqual(planDrop('local', { pane: 'remote', directory: '/home/bob/uploads/existing' }, dirs), { kind: 'upload', directory: '/home/bob/uploads/existing' });
  assert.deepEqual(planDrop('remote', { pane: 'local', directory: null }, dirs), { kind: 'download', directory: '/Users/me/Downloads' });
  assert.equal(planDrop('local', { pane: 'local', directory: null }, dirs), null, '同一侧内不做移动');
  assert.equal(planDrop('remote', null, dirs), null);
  assert.equal(planDrop('local', { pane: 'remote', directory: null }, { ...dirs, remote: '' }), null, '远程还没打开目录');
});

test('从 Finder 拖进来：只接受落在远程一侧', () => {
  assert.deepEqual(planDrop('finder', { pane: 'remote', directory: null }, dirs), { kind: 'upload', directory: '/home/bob/uploads' });
  assert.equal(planDrop('finder', { pane: 'local', directory: null }, dirs), null);
});

test('拖哪些：按住的行在选中里就拖整组选中，否则只拖这一行；「..」不能拖', () => {
  const files = [{ name: '..', path: '/home' }, { name: 'a', path: '/x/a' }, { name: 'b', path: '/x/b' }, { name: 'c', path: '/x/c' }];
  assert.deepEqual(dragItems(files[1], new Set(['/x/a', '/x/c']), files).map(f => f.name), ['a', 'c']);
  assert.deepEqual(dragItems(files[2], new Set(['/x/a', '/x/c']), files).map(f => f.name), ['b']);
  assert.deepEqual(dragItems(files[0], new Set(), files), []);
  assert.equal(dragLabel([files[1]]), 'a');
  assert.equal(dragLabel(files.slice(1)), '3 个项目');
});

test('Finder 落点坐标：macOS 上已是 CSS 像素，其他平台是物理像素', () => {
  assert.deepEqual(dropPointToCss({ x: 300, y: 120 }, true, 2), { x: 300, y: 120 });
  assert.deepEqual(dropPointToCss({ x: 600, y: 240 }, false, 2), { x: 300, y: 120 });
});

test('没收到 pointerup 的松手：在最后拖拽位置附近算松手，离得远（窗口外松手）算取消', () => {
  const drag = { active: true, lastX: 1112, lastY: 196 };
  assert.equal(releasedMove(drag, 1112, 196, 5), 'drop', '实测：WKWebView 丢了 pointerup，紧跟一个同位置的松开状态移动');
  assert.equal(releasedMove(drag, 1115, 199, 5), 'drop');
  assert.equal(releasedMove(drag, 300, 600, 5), 'cancel');
  assert.equal(releasedMove({ ...drag, active: false }, 1112, 196, 5), 'cancel', '还没真正拖起来');
});

test('落点没换（同一侧、同一个文件夹行）不算变化：指针在同一落点内移动不重渲染两侧列表', () => {
  const folder = '/home/bob/uploads/existing';
  assert.equal(sameDropTarget({ pane: 'remote', directory: null }, { pane: 'remote', directory: null }), true, '每次移动都是新对象，内容一样就是同一落点');
  assert.equal(sameDropTarget({ pane: 'remote', directory: folder }, { pane: 'remote', directory: folder }), true);
  assert.equal(sameDropTarget(null, null), true);
  assert.equal(sameDropTarget({ pane: 'remote', directory: null }, { pane: 'remote', directory: folder }), false, '移到文件夹行上');
  assert.equal(sameDropTarget({ pane: 'remote', directory: null }, { pane: 'local', directory: null }), false, '换了一侧');
  assert.equal(sameDropTarget(null, { pane: 'remote', directory: null }), false);
  assert.equal(sameDropTarget({ pane: 'remote', directory: null }, null), false);
});

test('拖拽浮层 store：位置变化通知订阅者，结束时清空；取消订阅后不再通知', () => {
  const store = createDragPreviewStore();
  assert.equal(store.get(), null);
  const seen = [];
  const unsubscribe = store.subscribe(() => seen.push(store.get()));
  const items = [{ name: 'a', path: '/x/a' }];
  store.set({ pane: 'local', items, x: 10, y: 20, destination: null });
  store.set({ pane: 'local', items, x: 12, y: 21, destination: '/home/bob' });
  store.set(null);
  unsubscribe();
  store.set({ pane: 'local', items, x: 1, y: 1, destination: null });
  assert.deepEqual(seen, [
    { pane: 'local', items, x: 10, y: 20, destination: null },
    { pane: 'local', items, x: 12, y: 21, destination: '/home/bob' },
    null,
  ]);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { visibleRowWindow } from '../ui/src/features/files/virtual-rows.ts';

const H = 33;

// spacer + 渲染行的总高度必须始终等于整张列表的高度，否则滚动条长度会跳。
function assertCoversList(win, count) {
  assert.equal(win.padTop + (win.end - win.start) * H + win.padBottom, count * H);
}

test('empty list renders nothing', () => {
  assert.deepEqual(visibleRowWindow(0, H, 0, 330, 10), { start: 0, end: 0, padTop: 0, padBottom: 0 });
});

test('top of a long list renders the viewport plus overscan below', () => {
  const win = visibleRowWindow(1000, H, 0, 330, 10);
  assert.deepEqual([win.start, win.end, win.padTop], [0, 20, 0]);
  assertCoversList(win, 1000);
});

test('middle of the list adds overscan on both sides, including partially visible rows', () => {
  const aligned = visibleRowWindow(1000, H, 100 * H, 330, 10);
  assert.deepEqual([aligned.start, aligned.end], [90, 120]);
  // 未对齐时顶部和底部各露出半行，底部那一行也必须渲染。
  const unaligned = visibleRowWindow(1000, H, 100 * H + 10, 330, 10);
  assert.deepEqual([unaligned.start, unaligned.end], [90, 121]);
  assertCoversList(unaligned, 1000);
});

test('scrolled to the bottom renders the tail with no bottom spacer', () => {
  const win = visibleRowWindow(1000, H, 1000 * H - 330, 330, 10);
  assert.deepEqual([win.start, win.end, win.padBottom], [980, 1000, 0]);
  assertCoversList(win, 1000);
});

test('overscan past both ends is clamped to the list', () => {
  assert.deepEqual(visibleRowWindow(5, H, 0, 10_000, 10), { start: 0, end: 5, padTop: 0, padBottom: 0 });
});

test('a zero-height viewport (hidden tab) still renders the overscan rows', () => {
  assert.deepEqual([visibleRowWindow(1000, H, 0, 0, 10).start, visibleRowWindow(1000, H, 0, 0, 10).end], [0, 10]);
  const mid = visibleRowWindow(1000, H, 100 * H, 0, 10);
  assert.deepEqual([mid.start, mid.end], [90, 110]);
  assertCoversList(mid, 1000);
});

test('a stale scrollTop past the end of a shrunk list pins the window to the tail', () => {
  const win = visibleRowWindow(50, H, 100_000, 330, 10);
  assert.equal(win.end, 50);
  assert.ok(win.end - win.start > 10, 'visible rows plus overscan, not an empty window');
  assertCoversList(win, 50);
});

test('negative scrollTop from macOS elastic scrolling is treated as the top', () => {
  assert.deepEqual(visibleRowWindow(1000, H, -40, 330, 10), visibleRowWindow(1000, H, 0, 330, 10));
});

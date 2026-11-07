import test from 'node:test';
import assert from 'node:assert/strict';
import { edgeExtension, pickEdgeBg } from '../ui/src/features/terminal/edge-bg.ts';

test('nvim: the status bar and every painted row reach the card edge, cell by cell', () => {
  // 6×4：首行编辑区，第 2 行 cursorline，末行状态栏（NORMAL 蓝 | 中间深 | 时钟蓝）；null = 默认底色
  const rows = [
    ['#222436', '#222436', '#222436', '#222436', '#222436', '#222436'],
    ['#2f334d', '#2f334d', '#2f334d', '#2f334d', '#2f334d', '#2f334d'],
    ['#222436', '#222436', '#222436', '#222436', '#222436', '#222436'],
    ['#82aaff', '#82aaff', '#1e2030', '#1e2030', '#82aaff', '#82aaff'],
  ];
  const ext = edgeExtension(6, 4, (x, y) => rows[y][x]);
  assert.deepEqual(ext.top, rows[0]);
  assert.deepEqual(ext.bottom, rows[3]);
  assert.deepEqual(ext.left, ['#222436', '#2f334d', '#222436', '#82aaff']);
  assert.deepEqual(ext.right, ['#222436', '#2f334d', '#222436', '#82aaff']);
});

test('a shell keeps its padding: partly colored rows are not extended', () => {
  // 提示符 / ls 着色只占半行：不往 padding 里抹
  const rows = [
    [null, null, null, null],
    ['#00ff74', '#00ff74', null, null],
    [null, null, '#ee5396', '#ee5396'],
    ['#78a9ff', '#78a9ff', null, null],
  ];
  const ext = edgeExtension(4, 4, (x, y) => rows[y][x]);
  assert.equal(ext.top, null);
  assert.equal(ext.bottom, null);
  assert.deepEqual(ext.left, [null, null, null, null]);
  assert.deepEqual(ext.right, [null, null, null, null]);
});

const grid = (cols, rows, paint) => (x, y) => paint(x, y, cols, rows);

test('vim with the cursor on the first line keeps its own background around the edges', () => {
  // 150×48：首行是 cursorline，末行是状态栏，其余是 vim 的根背景
  const colorAt = grid(150, 48, (x, y, _cols, rows) => {
    if (y === 0) return '#2f334d';
    if (y === rows - 1) return x < 10 ? '#82aaff' : '#1e2030';
    return '#222436';
  });
  assert.equal(pickEdgeBg(150, 48, colorAt), '#222436');
});

test('a plain shell screen stays on the default background', () => {
  assert.equal(pickEdgeBg(120, 40, () => '#1d1d1d'), '#1d1d1d');
});

test('a centered dialog that never touches the edges does not move the padding', () => {
  const colorAt = grid(100, 30, (x, y) => (x > 20 && x < 80 && y > 8 && y < 22 ? '#c0c0c0' : '#5f5fd7'));
  assert.equal(pickEdgeBg(100, 30, colorAt), '#5f5fd7');
});

test('when the two side columns disagree, all four edges decide', () => {
  // 左侧文件树一种底色、右侧编辑区另一种：两侧打平，四边里编辑区占多数
  const colorAt = grid(100, 30, x => (x < 30 ? '#111111' : '#222222'));
  assert.equal(pickEdgeBg(100, 30, colorAt), '#222222');
});

test('file tree + tabline + status bar: the edges disagree, the editor in the middle decides', () => {
  // 左侧文件树一种底色、右侧编辑区另一种；首行标签栏、末行状态栏各一种。两列打平，四边谁也不过半。
  const colorAt = grid(120, 40, (x, y, _cols, rows) => {
    if (y === 0) return '#3b4261';
    if (y === rows - 1) return '#82aaff';
    return x < 30 ? '#1a1b26' : '#222436';
  });
  assert.equal(pickEdgeBg(120, 40, colorAt), '#222436');
});

test('no color reaches a majority anywhere: no opinion', () => {
  const colorAt = grid(40, 40, (x, y) => ['#111111', '#222222', '#333333'][(x + y) % 3]);
  assert.equal(pickEdgeBg(40, 40, colorAt), null);
  assert.equal(pickEdgeBg(0, 0, () => '#000000'), null);
});

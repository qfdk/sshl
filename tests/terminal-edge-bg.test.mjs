import test from 'node:test';
import assert from 'node:assert/strict';
import { pickEdgeBg } from '../ui/src/features/terminal/edge-bg.ts';

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

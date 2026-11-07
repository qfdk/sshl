import test from 'node:test';
import assert from 'node:assert/strict';
import { centerOffset } from '../ui/src/features/terminal/screen-fit.ts';

test('the leftover after fitting whole cells is split between both sides', () => {
  // 可用 600px，渲染区 581px（整行排下后差不到一行）：上面 9px、下面 10px，而不是上面整整 19px
  assert.equal(centerOffset(600, 581), 9);
  assert.equal(centerOffset(812.5, 800), 6);
});

test('never pushes the screen outside the padding box', () => {
  assert.equal(centerOffset(500, 500), 0);
  assert.equal(centerOffset(500, 503), 0);
  assert.equal(centerOffset(0, 0), 0);
});

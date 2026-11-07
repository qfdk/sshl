import test from 'node:test';
import assert from 'node:assert/strict';
import { isCursorReset } from '../ui/src/features/terminal/cursor-style.ts';

// DECSCUSR：CSI Ps SP q。nvim 退出时发 ESC[0 q，要回到终端自己的默认光标，而不是 xterm 5.3 当成的闪烁方块
test('Ps 0 or no parameter means "back to the terminal default cursor"', () => {
  assert.equal(isCursorReset([0]), true);
  assert.equal(isCursorReset([]), true);
  assert.equal(isCursorReset([[0]]), true);
});

test('explicit shapes are left to xterm (vim switching block / bar between modes)', () => {
  for (const ps of [1, 2, 3, 4, 5, 6]) assert.equal(isCursorReset([ps]), false, `Ps ${ps}`);
});

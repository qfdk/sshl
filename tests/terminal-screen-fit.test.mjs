import test from 'node:test';
import assert from 'node:assert/strict';
import { centerOffset, snapMargin } from '../ui/src/features/terminal/screen-fit.ts';

// FitAddon 的算法：父元素高度取整（parseInt）减 padding，再按行高向下取整
const fitRows = (cardHeight, padding, cell) => Math.floor((Math.trunc(cardHeight) - padding) / cell);

test('the card gives up the part of a row that does not fit, so the frame is the padding on every side', () => {
  for (const [available, cell] of [[900, 18.5], [900, 17.25], [1022, 19], [640.5, 17.5]]) {
    const margin = snapMargin(available, 8, cell);
    const card = available - margin;
    const rows = fitRows(card, 8, cell);
    assert.equal(rows, Math.floor((available - 8) / cell), `keeps every row that fits (${available}/${cell})`);
    assert.ok(card - 8 - rows * cell < 1, `less than a pixel left inside the card (${available}/${cell})`);
    assert.ok(margin >= 0 && margin < cell, `margin is under one row (${available}/${cell})`);
  }
});

test('nothing to snap while the terminal is hidden or not measured yet', () => {
  assert.equal(snapMargin(0, 8, 18), 0);
  assert.equal(snapMargin(900, 8, 0), 0);
});

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

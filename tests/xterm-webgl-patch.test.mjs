import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// xterm-addon-webgl 0.16 的背景矩形缓冲上限少一块：满屏每格背景都不同（终端里看图、棋盘色块）时
// drawElementsInstanced 越界，整个终端一片黑。patches/xterm-addon-webgl@0.16.0.patch 回移了上游修复，
// 这里防止重装依赖后补丁没打上。
const lib = readFileSync(new URL('../node_modules/xterm-addon-webgl/lib/xterm-addon-webgl.js', import.meta.url), 'utf8');

test('webgl background rectangles have room for one per cell plus the reserved slot', () => {
  assert.ok(lib.includes('attributes.length<t+8&&(e.attributes=(0,n.expandFloat32Array)(e.attributes,(this._terminal.rows*this._terminal.cols+1)*8))'));
  assert.ok(!lib.includes('this._terminal.rows*this._terminal.cols*8'));
});

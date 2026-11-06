import test from 'node:test';
import assert from 'node:assert/strict';
import { pointerMoveStep, releasedMove } from '../ui/src/lib/pointer-release.ts';
import { releasedMove as fileDragReleasedMove } from '../ui/src/features/files/file-drag.ts';

const LEFT = 1;
const RIGHT = 2;
const pressed = (x, y) => ({ active: false, startX: x, startY: y, lastX: x, lastY: y });
const move = (x, y, buttons) => ({ clientX: x, clientY: y, buttons });
const moved = (drag, event) => Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
const step = (drag, event, threshold = 6) => pointerMoveStep(drag, event, moved(drag, event), threshold);

test('文件管理仍从 file-drag 拿到同一个 releasedMove', () => {
  assert.equal(fileDragReleasedMove, releasedMove);
});

test('实测复现：双击丢了 pointerup，之后不按键移动 20px，不能进入拖拽', () => {
  // SSHL 刚切到前台时双击侧栏连接：WKWebView 收到 pointerdown、没收到 pointerup，按下状态一直留着
  const drag = pressed(120, 300);
  for (let dx = 1; dx <= 20; dx++) {
    assert.equal(step(drag, move(120 + dx, 300, 0)), 'cancel', `左键已松开的第 ${dx}px 移动只取消`);
  }
});

test('还没拖起来、左键已松开：阈值内也取消，不等到越过阈值', () => {
  assert.equal(step(pressed(10, 10), move(10, 10, 0)), 'cancel');
  assert.equal(step(pressed(10, 10), move(12, 11, 0)), 'cancel');
});

test('按住左键：阈值内算点击（忽略），越过阈值进入拖拽', () => {
  const drag = pressed(10, 10);
  assert.equal(step(drag, move(13, 10, LEFT)), 'ignore');
  assert.equal(step(drag, move(16, 10, LEFT)), 'drag');
  assert.equal(step(drag, move(16, 10, LEFT | RIGHT)), 'drag', '左键还按着，多按了别的键也照常拖');
});

test('拖拽中按住移动照常拖，回到按下点附近也不算点击', () => {
  const drag = { ...pressed(10, 10), active: true, lastX: 200, lastY: 80 };
  assert.equal(step(drag, move(200, 80, LEFT)), 'drag');
  assert.equal(step(drag, move(11, 10, LEFT)), 'drag');
});

test('拖拽中丢了 pointerup：就在最后位置附近算松手，离得远算取消', () => {
  const drag = { ...pressed(10, 10), active: true, lastX: 200, lastY: 80 };
  assert.equal(step(drag, move(200, 80, 0)), 'drop');
  assert.equal(step(drag, move(203, 83, 0)), 'drop');
  assert.equal(step(drag, move(400, 80, 0)), 'cancel', '在窗口外松手、指针后来才回来');
  assert.equal(step(drag, move(200, 80, RIGHT)), 'drop', '只看左键：左键松了就是松手');
});

test('拖起阈值的距离由调用方量：分组排序只看纵向，横向晃动不进入拖拽', () => {
  const drag = pressed(10, 10);
  const event = move(40, 12, LEFT);
  assert.equal(pointerMoveStep(drag, event, Math.abs(event.clientY - drag.startY), 6), 'ignore');
  assert.equal(pointerMoveStep(drag, move(40, 12, 0), Math.abs(event.clientY - drag.startY), 6), 'cancel');
});

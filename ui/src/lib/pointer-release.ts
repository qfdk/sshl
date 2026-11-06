// 指针拖拽丢了 pointerup 的兜底，会话标签、连接列表、分组树和文件管理共用。
// 只放纯判定，不引入别的模块：会话标签、连接列表在首屏主包里，文件管理是懒加载的。

/**
 * 拖拽中收到「左键已松开」的指针移动、却没收到 pointerup 时怎么办。WKWebView 拖拽后常丢 pointerup / mouseup，
 * 紧接着补一个松开状态的 pointermove：就在最后的拖拽位置附近 → 当作在这里松手（drop）；
 * 离得远（在窗口外松手、指针后来才回来）→ 取消，免得误传，也免得提示卡在屏幕上。
 */
export function releasedMove(drag: { active: boolean; lastX: number; lastY: number }, x: number, y: number, tolerance: number) {
  return drag.active && Math.hypot(x - drag.lastX, y - drag.lastY) <= tolerance ? 'drop' : 'cancel';
}

export type PointerMoveStep = 'ignore' | 'drag' | 'drop' | 'cancel';

/**
 * 按下后收到 pointermove 时这一步怎么走：
 * - 左键已松开却没收到 pointerup → 见 releasedMove。还没拖起来时一律取消：否则松开后再不按键移动鼠标
 *   就会进入拖拽，下一次点击的 pointerup 被当成拖完松手，把东西挪走（实测：刚切到前台时双击侧栏连接）。
 * - 还没拖起来、离按下点不到阈值 → ignore，算点击。
 * - 否则 drag：进入或继续拖拽。
 * moved 是离按下点的距离，由调用方量（分组排序只看纵向）；threshold 也是松手判定的容差。
 */
export function pointerMoveStep(
  drag: { active: boolean; lastX: number; lastY: number },
  event: { clientX: number; clientY: number; buttons: number },
  moved: number,
  threshold: number,
): PointerMoveStep {
  if ((event.buttons & 1) === 0) return releasedMove(drag, event.clientX, event.clientY, threshold);
  return drag.active || moved >= threshold ? 'drag' : 'ignore';
}

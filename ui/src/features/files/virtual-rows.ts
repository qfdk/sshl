export type RowWindow = {
  start: number;
  end: number;
  padTop: number;
  padBottom: number;
};

// 计算等高行列表应渲染的区间 [start, end)，以及上下 spacer 的高度。
// scrollTop 是相对第一行顶部的偏移；macOS 回弹滚动时可能为负，按 0 处理。
export function visibleRowWindow(count: number, rowHeight: number, scrollTop: number, viewportHeight: number, overscan: number): RowWindow {
  if (count <= 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  if (!(rowHeight > 0)) return { start: 0, end: count, padTop: 0, padBottom: 0 };
  const top = Math.max(0, scrollTop);
  const view = Math.max(0, viewportHeight);
  let first = Math.floor(top / rowHeight);
  let last = Math.ceil((top + view) / rowHeight);
  // 列表变短后 scrollTop 可能还停在旧的位置（越界），把窗口贴到末尾，不渲染出一片空白。
  if (last > count) {
    const span = last - first;
    last = count;
    first = Math.max(0, count - span);
  }
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, last + overscan);
  return { start, end, padTop: start * rowHeight, padBottom: (count - end) * rowHeight };
}

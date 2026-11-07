// 渲染区按整 cell 排版，总比可用区域小一点（不足一行 / 一列）。余量平分到两侧时，渲染区该偏移多少。
export function centerOffset(available: number, used: number) {
  return Math.floor(Math.max(0, Math.round(available - used)) / 2);
}

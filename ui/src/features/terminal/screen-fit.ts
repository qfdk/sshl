// 渲染区按整 cell 排版，总比可用区域小一点（不足一行 / 一列）。余量平分到两侧时，渲染区该偏移多少。
export function centerOffset(available: number, used: number) {
  return Math.floor(Math.max(0, Math.round(available - used)) / 2);
}

// 终端卡片吸附整行：卡片底部让出多少，卡片里正好排下整数行，四边只剩 padding。
// 让出的不足一行交给卡片外的浅色背景，而不是在深色卡片里多出一道边。
// FitAddon 把父元素高度 parseInt 取整，所以卡片高度向上取整到整像素，免得少排一行。
export function snapMargin(available: number, padding: number, cell: number) {
  if (!(available > padding && cell > 0)) return 0;
  const rows = Math.floor((available - padding) / cell + 1e-6);
  return Math.max(0, available - (Math.ceil(rows * cell - 1e-6) + padding));
}

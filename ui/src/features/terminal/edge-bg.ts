type ColorAt = (x: number, y: number) => string | null;

function majority(colors: (string | null)[], strict: boolean) {
  const counts = new Map<string, number>();
  for (const color of colors) if (color) counts.set(color, (counts.get(color) || 0) + 1);
  let best: string | null = null, max = 0;
  for (const [color, count] of counts) if (count > max) { best = color; max = count; }
  if (!best) return null;
  return (strict ? max * 2 > colors.length : max * 2 >= colors.length) ? best : null;
}

// 终端 padding 取什么底色：看贴着 padding 的边缘 cell。
// 先看左右两列：光标行、标签栏、状态栏这类横贯整行的条带在两列里各只占一格，两列最能代表 TUI 的根背景。
// 只看四条边时，宽屏上首末两行（各 cols 格）压过两侧：vim 光标停在首行时首行是 cursorline 色、末行是状态栏，
// 根背景只剩约 24%，padding 回落默认色，四周露出一圈黑框。
// 两列凑不出严格过半（左右分属不同面板）再按四条边取众数（≥50%）。
export function pickEdgeBg(cols: number, rows: number, colorAt: ColorAt) {
  if (!cols || !rows) return null;
  const sides: (string | null)[] = [];
  for (let y = 0; y < rows; y++) sides.push(colorAt(0, y), colorAt(cols - 1, y));
  const side = majority(sides, true);
  if (side) return side;
  const edges = [...sides];
  for (let x = 0; x < cols; x++) edges.push(colorAt(x, 0), colorAt(x, rows - 1));
  return majority(edges, false);
}

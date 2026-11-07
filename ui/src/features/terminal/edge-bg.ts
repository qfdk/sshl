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
// 四条边也定不下来（左边文件树、右边编辑区、首行标签栏、末行状态栏各一种色）时，取屏幕中央的主色：
// 中央是正文区，padding 跟正文同色，回落默认色反而四周一圈黑框。
export function pickEdgeBg(cols: number, rows: number, colorAt: ColorAt) {
  if (!cols || !rows) return null;
  const sides: (string | null)[] = [];
  for (let y = 0; y < rows; y++) sides.push(colorAt(0, y), colorAt(cols - 1, y));
  const side = majority(sides, true);
  if (side) return side;
  const edges = [...sides];
  for (let x = 0; x < cols; x++) edges.push(colorAt(x, 0), colorAt(x, rows - 1));
  const edge = majority(edges, false);
  if (edge) return edge;
  // 中央一半区域隔格取样，够定主色，也免得每帧读上千个 cell
  const center: (string | null)[] = [];
  for (let y = Math.floor(rows / 4); y < Math.ceil((rows * 3) / 4); y += 2) {
    for (let x = Math.floor(cols / 4); x < Math.ceil((cols * 3) / 4); x += 2) center.push(colorAt(x, y));
  }
  return majority(center, false);
}

export type EdgeExtension = {
  /** 首行每格的底色（整行都有底色时），否则 null */
  top: string[] | null;
  /** 末行每格的底色（整行都有底色时），否则 null */
  bottom: string[] | null;
  /** 每行最左一格的底色（该行整行都有底色时），否则 null */
  left: (string | null)[];
  /** 每行最右一格的底色（该行整行都有底色时），否则 null */
  right: (string | null)[];
};

// padding 延伸边缘格子的底色（同 Ghostty 的 window-padding-color = extend）：状态栏、标签栏贴到卡片边，
// cursorline 这类整行高亮延伸到左右边，padding 里不再夹一道根背景色的缝。
// 只延伸「整行都有底色」的行：shell 提示符、ls 着色只占半行，抹进 padding 反而难看。colorAt 对默认底色返回 null。
export function edgeExtension(cols: number, rows: number, colorAt: ColorAt): EdgeExtension {
  const left: (string | null)[] = [];
  const right: (string | null)[] = [];
  let top: string[] | null = null;
  let bottom: string[] | null = null;
  for (let y = 0; y < rows; y++) {
    const edgeRow = y === 0 || y === rows - 1;
    const colors: string[] = [];
    let full = cols > 0;
    for (let x = 0; x < cols; x++) {
      const color = colorAt(x, y);
      if (!color) { full = false; break; }
      if (edgeRow) colors.push(color);
      else if (x === 0 || x === cols - 1) colors[x === 0 ? 0 : 1] = color;
    }
    left.push(full ? colors[0] : null);
    right.push(full ? colors[colors.length - 1] : null);
    if (full && y === 0) top = colors;
    if (full && y === rows - 1) bottom = colors;
  }
  return { top, bottom, left, right };
}

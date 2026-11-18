//! 磁盘页右边的矩形树图（同 Mole、DaisyDisk）：这一层每一项一块，面积就是占用，一眼看出谁大。
//! 排法是 squarified（Bruls、Huizing、van Wijk，2000）：从大到小沿短边一排排铺，一排里一直加，加到再加一块会让最扁的那块更扁为止。
//! 块都接近正方形，名字放得下。太小的项（不到这一层的 [`MIN_SHARE`]，或排在第 [`MAX_TILES`] 名之后）合成一块「其余 N 项」：
//! 一堆细条既看不清也点不中。

use std::cmp::Reverse;

/// 单独成块的最多这么多。
pub const MAX_TILES: usize = 40;
/// 不到这一层总量的这么多就并进「其余」。
pub const MIN_SHARE: f64 = 0.004;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// 一块代表什么：这一层的第几行，或者合起来的那些（一共几项）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Row(usize),
    Rest { count: usize },
}

/// 哪些行单独成块、哪些合起来：(块, 大小)，从大到小。`rows` 是这一层每一行的 (大小, 代表几项)：
/// 列表里已经合成一行的「其余 N 个文件」代表 N 项。大小是 0 的不画。只剩一项要合的话不合，它自己一块。
pub fn items(rows: &[(u64, usize)]) -> Vec<(Item, u64)> {
    let total: u64 = rows.iter().map(|row| row.0).sum();
    let floor = ((total as f64 * MIN_SHARE) as u64).max(1);
    let mut order: Vec<usize> = (0..rows.len()).filter(|&ix| rows[ix].0 > 0).collect();
    order.sort_by_key(|&ix| Reverse(rows[ix].0));
    let mut out = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    for (rank, &ix) in order.iter().enumerate() {
        if rank < MAX_TILES && rows[ix].0 >= floor {
            out.push((Item::Row(ix), rows[ix].0));
        } else {
            rest.push(ix);
        }
    }
    match rest.as_slice() {
        [] => {}
        [only] => out.push((Item::Row(*only), rows[*only].0)),
        many => {
            let count = many.iter().map(|&ix| rows[ix].1).sum();
            out.push((Item::Rest { count }, many.iter().map(|&ix| rows[ix].0).sum()));
        }
    }
    // 稳定排序：一样大时单独的项在「其余」前面
    out.sort_by_key(|(_, size)| Reverse(*size));
    out
}

/// 把 `sizes`（从大到小）铺满 `bounds`：每块的面积和大小成正比。大小是 0 的得到一个没有面积的块。
pub fn squarify(sizes: &[u64], bounds: Rect) -> Vec<Rect> {
    let mut out = vec![Rect { x: bounds.x, y: bounds.y, w: 0., h: 0. }; sizes.len()];
    let total: f64 = sizes.iter().map(|&size| size as f64).sum();
    if total <= 0. || bounds.w <= 0. || bounds.h <= 0. {
        return out;
    }
    let scale = f64::from(bounds.w) * f64::from(bounds.h) / total;
    let areas: Vec<f64> = sizes.iter().map(|&size| size as f64 * scale).collect();
    let (mut x, mut y, mut w, mut h) = (f64::from(bounds.x), f64::from(bounds.y), f64::from(bounds.w), f64::from(bounds.h));
    let mut start = 0;
    while start < areas.len() {
        if areas[start] <= 0. {
            start += 1;
            continue;
        }
        let short = w.min(h);
        // 这一排加到哪里：再加一块会让最扁的更扁就停
        let (mut end, mut sum) = (start + 1, areas[start]);
        let mut worst = worst_ratio(&areas[start..end], sum, short);
        while end < areas.len() && areas[end] > 0. {
            let next = worst_ratio(&areas[start..=end], sum + areas[end], short);
            if next > worst {
                break;
            }
            worst = next;
            sum += areas[end];
            end += 1;
        }
        let last = end == areas.len() || areas[end..].iter().all(|&area| area <= 0.);
        if w >= h {
            // 宽的：这一排竖着放在左边
            let thick = if last { w } else { sum / h };
            let mut at = y;
            for ix in start..end {
                let size = areas[ix] / thick;
                out[ix] = rect(x, at, thick, size);
                at += size;
            }
            x += thick;
            w -= thick;
        } else {
            let thick = if last { h } else { sum / w };
            let mut at = x;
            for ix in start..end {
                let size = areas[ix] / thick;
                out[ix] = rect(at, y, size, thick);
                at += size;
            }
            y += thick;
            h -= thick;
        }
        start = end;
    }
    out
}

/// 块里的字：名字 12pt、一行 16pt 高；大小 11pt、一行 14pt 高；两段之间空 2pt。名字前的图标 12pt，和名字隔 5pt。
pub const NAME_FONT: f32 = 12.;
pub const NAME_LINE: f32 = 16.;
pub const SIZE_FONT: f32 = 11.;
pub const SIZE_LINE: f32 = 14.;
pub const LINE_GAP: f32 = 2.;
pub const ICON: f32 = 12.;
pub const ICON_GAP: f32 = 5.;
/// 写字的块左右的内边距，和块的边框（选中时才有颜色，一直占着位置）。
pub const PAD: f32 = 6.;
pub const BORDER: f32 = 1.;
/// 一段没有断点的字（「Library」、一串哈希）一行放不下时，块里写字的地方起码这么宽（一行十几个字）才在字中间断开；
/// 再窄断出来是「Libra / ry」，不如不写。
pub const WRAP_MIN_WIDTH: f32 = 96.;
pub const MAX_NAME_LINES: usize = 3;

/// 宽 `w` 的块里能写字的地方多宽：去掉边框和左右内边距。
pub fn text_width(w: f32) -> f32 {
    w - 2. * (BORDER + PAD)
}

/// 把名字折成每行不超过 `width` 的几行（`measure` 量一段字多宽），写不下返回空。同访达，只在自然断开的地方折：
/// `.` `-` `_` 空格 `/` 之后（开头的点不算：不出一行只有「.」的），中文这类宽字之间——「com.tencent.qq」折成「com.tencent.」「qq」。
/// 一段没有断点的字一行放不下时，宽过 [`WRAP_MIN_WIDTH`] 才在字中间断开，不然写不下。行数多了也照样返回，由 [`label`] 判断。
pub fn wrap_name(name: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    if width <= 0. {
        return Vec::new();
    }
    if measure(name) <= width {
        return vec![name.to_string()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for piece in pieces(name) {
        let joined = format!("{line}{piece}");
        if measure(joined.trim()) <= width {
            line = joined;
            continue;
        }
        if !line.trim().is_empty() {
            lines.push(line.trim().to_string());
        }
        line = String::new();
        if measure(piece.trim()) <= width {
            line = piece.to_string();
        } else if width >= WRAP_MIN_WIDTH {
            for ch in piece.chars() {
                let joined = format!("{line}{ch}");
                if line.is_empty() || measure(&joined) <= width {
                    line = joined;
                } else {
                    lines.push(std::mem::replace(&mut line, ch.to_string()));
                }
            }
        } else {
            return Vec::new();
        }
        if lines.len() > MAX_NAME_LINES {
            return lines;
        }
    }
    if !line.trim().is_empty() {
        lines.push(line.trim().to_string());
    }
    lines
}

/// 名字按能断开的地方切成几段，每段带着结尾的断点。
fn pieces(name: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (ix, ch) in name.char_indices() {
        let end = ix + ch.len_utf8();
        // 中日韩的字（和全角标点）每个字前后都能断开
        let wide = ch >= '\u{2E80}';
        if wide && ix > start {
            out.push(&name[start..ix]);
            start = ix;
        }
        if wide || (ix > start && matches!(ch, '.' | '-' | '_' | ' ' | '/')) {
            out.push(&name[start..end]);
            start = end;
        }
    }
    if start < name.len() {
        out.push(&name[start..]);
    }
    out
}

/// 块里写什么。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Label {
    /// 名字写几行；0 = 不写字，悬停看气泡。
    pub lines: usize,
    pub icon: bool,
    pub size: bool,
}

/// 一块（`w` × `h`，含边框）里写什么。字要整段写得下才写：名字不截成「Libr…」，大小不折成「3.3 / GB」。
/// `name` / `size` 是名字、大小写成一行有多宽，`lines` 是名字按 [`wrap_name`] 折成了几行（0 = 宽度上就写不下）。
/// 名字写不下时大小也不写：光一个大小看不出是谁。
pub fn label(w: f32, h: f32, has_icon: bool, name: f32, lines: usize, size: f32) -> Label {
    let (inner_w, inner_h) = (text_width(w), h - 2. * BORDER);
    let name_h = lines as f32 * NAME_LINE;
    if inner_w <= 0. || lines == 0 || lines > MAX_NAME_LINES || name_h > inner_h {
        return Label::default();
    }
    Label {
        lines,
        icon: has_icon && lines == 1 && name + ICON + ICON_GAP <= inner_w,
        size: size <= inner_w && name_h + LINE_GAP + SIZE_LINE <= inner_h,
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x: x as f32, y: y as f32, w: w as f32, h: h as f32 }
}

/// 一排（总面积 `sum`，沿长为 `side` 的边铺）里最扁的那块有多扁：长边 / 短边，1 是正方形。
fn worst_ratio(row: &[f64], sum: f64, side: f64) -> f64 {
    let (side2, sum2) = (side * side, sum * sum);
    row.iter().map(|&area| (side2 * area / sum2).max(sum2 / (side2 * area))).fold(1., f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Rect = Rect { x: 10., y: 20., w: 600., h: 400. };

    /// 差得很少（f32：几十万的面积加起来差个零点零几是正常的）。
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 0.01_f32.max(b.abs() * 1e-5)
    }

    fn overlap(a: &Rect, b: &Rect) -> f32 {
        let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
        let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
        if w > 0. && h > 0. { w * h } else { 0. }
    }

    /// 面积按大小分、铺满不出界、互不重叠。
    #[test]
    fn tiles_fill_the_area_in_proportion_without_overlap() {
        let sizes = [5000, 3000, 1200, 900, 400, 250, 100, 50];
        let total: u64 = sizes.iter().sum();
        let tiles = squarify(&sizes, BOUNDS);
        let area = BOUNDS.w * BOUNDS.h;
        for (tile, &size) in tiles.iter().zip(&sizes) {
            assert!(close(tile.w * tile.h, area * size as f32 / total as f32), "{tile:?} 该占 {size}/{total}");
            assert!(tile.x >= BOUNDS.x - 0.01 && tile.y >= BOUNDS.y - 0.01, "{tile:?}");
            assert!(tile.x + tile.w <= BOUNDS.x + BOUNDS.w + 0.01 && tile.y + tile.h <= BOUNDS.y + BOUNDS.h + 0.01, "{tile:?}");
        }
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                assert!(overlap(a, b) < 0.01, "{a:?} 和 {b:?} 重叠了");
            }
        }
        let covered: f32 = tiles.iter().map(|tile| tile.w * tile.h).sum();
        assert!(close(covered, area), "铺满：{covered} / {area}");
    }

    /// squarified 的意义：一样大的几项在正方形里排成方块，不是一排细条。
    #[test]
    fn equal_items_become_squares_not_strips() {
        let tiles = squarify(&[1; 9], Rect { x: 0., y: 0., w: 300., h: 300. });
        for tile in &tiles {
            let ratio = tile.w.max(tile.h) / tile.w.min(tile.h);
            assert!(ratio < 1.6, "太扁了：{tile:?}");
        }
        // 一大一小：大的占左边一整条
        let tiles = squarify(&[3, 1], Rect { x: 0., y: 0., w: 400., h: 100. });
        assert_eq!(tiles[0], Rect { x: 0., y: 0., w: 300., h: 100. });
        assert_eq!(tiles[1], Rect { x: 300., y: 0., w: 100., h: 100. });
    }

    #[test]
    fn nothing_to_draw_gives_empty_tiles() {
        assert!(squarify(&[], BOUNDS).is_empty());
        assert!(squarify(&[0, 0], BOUNDS).iter().all(|tile| tile.w == 0. && tile.h == 0.));
        assert!(squarify(&[5], Rect { w: 0., ..BOUNDS }).iter().all(|tile| tile.w * tile.h == 0.), "没有地方");
        let tiles = squarify(&[5, 0, 3], BOUNDS);
        assert_eq!(tiles[1].w * tiles[1].h, 0., "0 大小的没有面积");
        assert!(close(tiles[0].w * tiles[0].h + tiles[2].w * tiles[2].h, BOUNDS.w * BOUNDS.h));
    }

    /// 小的合成一块「其余」，按它的总大小排进去；只剩一项要合的话不合。列表里本来就合成一行的「其余 N 个文件」算 N 项。
    #[test]
    fn small_items_are_folded_into_one_rest_tile() {
        // 总量 10000，不到 0.4%（40）的合起来
        let rows = [(6000, 1), (30, 1), (3000, 1), (20, 50), (900, 1), (10, 1), (0, 1)];
        let got = items(&rows);
        assert_eq!(got, [(Item::Row(0), 6000), (Item::Row(2), 3000), (Item::Row(4), 900), (Item::Rest { count: 52 }, 60)]);

        let rows = [(6000, 1), (3000, 1), (5, 1)];
        assert_eq!(items(&rows), [(Item::Row(0), 6000), (Item::Row(1), 3000), (Item::Row(2), 5)], "只有一项小的：不合");
        assert!(items(&[(0, 1)]).is_empty());
    }

    /// 量字：英文一个字 6 宽，中文 12 宽。
    fn measure(text: &str) -> f32 {
        text.chars().map(|ch| if ch.is_ascii() { 6. } else { 12. }).sum()
    }

    fn wrap(name: &str, width: f32) -> Vec<String> {
        wrap_name(name, width, measure)
    }

    /// 截图里右下角那块：47 宽（写字的地方 33），写不下「Library」（42）——以前截成「Libr…」、大小折成「3.3 / GB」，现在不写字。
    #[test]
    fn narrow_tile_writes_nothing_rather_than_a_cut_name() {
        let width = text_width(47.);
        assert_eq!(width, 33.);
        assert!(wrap("Library", width).is_empty(), "没有断点、块又窄：不在字中间断开");
        assert_eq!(label(47., 90., true, measure("Library"), 0, 33.), Label::default());
        // 名字写不下时，大小写得下也不单独写
        assert_eq!(label(47., 90., true, 60., 0, 20.), Label::default());
    }

    /// 只在自然断开的地方折（同访达）：点、横线、下划线、空格、斜杠之后，中文字之间；开头的点不算断点。
    #[test]
    fn names_wrap_at_natural_breaks() {
        assert_eq!(wrap("usr", 74.), ["usr"], "写得下就一行");
        assert_eq!(wrap("com.tencent.qq", 76.), ["com.tencent.", "qq"]);
        assert_eq!(wrap("Application Support", 80.), ["Application", "Support"], "空格不留在行尾");
        assert_eq!(wrap("node_modules", 50.), ["node_", "modules"]);
        assert_eq!(wrap("微信文件存储", 40.), ["微信文", "件存储"]);
        assert_eq!(wrap("备份 2026-11-18", 45.), ["备份", "2026-", "11-18"]);
        assert!(wrap(".PreviousSystemInformation", 60.).is_empty(), "开头的点后面不断开：不出一行只有「.」的");
        assert_eq!(wrap(".config.old", 50.), [".config.", "old"]);
        assert!(wrap("anything", 0.).is_empty());
    }

    /// 一段没有断点的字一行放不下：块够宽才在字中间断开（一串哈希），不够宽不写。
    #[test]
    fn unbreakable_runs_split_only_in_wide_tiles() {
        let hash = "0123456789abcdef0123456789abcdef";
        assert_eq!(wrap(hash, 120.), ["0123456789abcdef0123", "456789abcdef"]);
        assert!(wrap(hash, 90.).is_empty());
        // 前面能断开的照样在断点断开，只有放不下的那段在字中间断
        assert_eq!(wrap(&format!("cache-{hash}"), 120.), ["cache-", "0123456789abcdef0123", "456789abcdef"]);
    }

    /// 写得下就整段写：宽了带图标，高了带大小。
    #[test]
    fn whole_name_and_size_when_they_fit() {
        // 「usr」：88 宽，名字 20、大小 33
        assert_eq!(label(88., 60., true, 20., 1, 33.), Label { lines: 1, icon: true, size: true });
        // 图标放不下就只写名字（36 宽的里面写得下 30 的名字，加图标要 47）
        assert_eq!(label(50., 60., true, 30., 1, 30.), Label { lines: 1, icon: false, size: true });
        // 大小比名字宽、放不下（38 宽的里面写 40）：只写名字，不折成两行
        assert_eq!(label(52., 60., true, 20., 1, 40.), Label { lines: 1, icon: true, size: false });
        // 不够高写大小
        assert_eq!(label(88., 24., true, 20., 1, 33.), Label { lines: 1, icon: true, size: false });
        // 连一行名字都不够高
        assert_eq!(label(88., 15., true, 20., 1, 33.), Label::default());
        // 合起来的那块（「其余 N 项」）没有图标
        assert_eq!(label(88., 60., false, 20., 1, 33.), Label { lines: 1, icon: false, size: true });
    }

    /// 折成几行的名字：不带图标；折完放不下或行数太多就不写。
    #[test]
    fn wrapped_names_need_room_for_every_line() {
        assert_eq!(label(92., 140., true, 84., 2, 33.), Label { lines: 2, icon: false, size: true }, "com.tencent.qq 那块");
        assert_eq!(label(200., 40., true, 300., 2, 33.), Label { lines: 2, icon: false, size: false }, "两行名字够高，加大小不够");
        assert_eq!(label(200., 30., true, 300., 2, 33.), Label::default(), "两行都不够高");
        assert_eq!(label(200., 200., true, 900., MAX_NAME_LINES + 1, 33.), Label::default(), "行数太多");
    }

    /// 项太多：前 MAX_TILES 名单独成块，其余合起来；合起来的比别的大时排在前面。
    #[test]
    fn many_items_keep_only_the_largest_tiles() {
        let rows: Vec<(u64, usize)> = (0..100).map(|_| (100, 1)).collect();
        let got = items(&rows);
        assert_eq!(got.len(), MAX_TILES + 1);
        assert_eq!(got[0], (Item::Rest { count: 100 - MAX_TILES }, 100 * (100 - MAX_TILES) as u64), "合起来的最大，排第一");
        assert!(got[1..].iter().all(|(item, size)| matches!(item, Item::Row(_)) && *size == 100));
    }
}

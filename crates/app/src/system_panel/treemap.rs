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

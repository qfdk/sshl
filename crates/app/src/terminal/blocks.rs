//! 方块字符（U+2580–U+259F：▀ ▄ █ ▌ ▐ ▛ ▜ ░ 这些）按格子自己画成矩形，不用字体的字形（同 Ghostty 的 sprite font）。
//! 字体里 █ 的宽度是字体自己的字宽（JetBrains Mono 13pt 约 7.8pt），格子按设备像素取整后更宽，相邻两格之间漏一条缝：
//! Claude Code 标志（▐▛███▜▌）上的竖线就是这么来的。自己画的话边正好落在格子边上，拼起来严丝合缝。

use gpui::{Bounds, Pixels, point, px};

/// 一个方块字符：格子里要涂满的几块（以 1/8 格为单位：左、上、右、下），和不透明度（░ ▒ ▓ 是整格加透明度）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    rects: &'static [[u8; 4]],
    pub alpha: f32,
}

const fn solid(rects: &'static [[u8; 4]]) -> Block {
    Block { rects, alpha: 1. }
}

const fn shade(alpha: f32) -> Block {
    Block { rects: &[[0, 0, 8, 8]], alpha }
}

/// U+2580 起的 32 个字符，顺序同 Unicode。
static BLOCKS: [Block; 32] = [
    solid(&[[0, 0, 8, 4]]), // ▀ 上半
    solid(&[[0, 7, 8, 8]]), // ▁ 下 1/8
    solid(&[[0, 6, 8, 8]]), // ▂ 下 1/4
    solid(&[[0, 5, 8, 8]]), // ▃ 下 3/8
    solid(&[[0, 4, 8, 8]]), // ▄ 下半
    solid(&[[0, 3, 8, 8]]), // ▅ 下 5/8
    solid(&[[0, 2, 8, 8]]), // ▆ 下 3/4
    solid(&[[0, 1, 8, 8]]), // ▇ 下 7/8
    solid(&[[0, 0, 8, 8]]), // █ 整格
    solid(&[[0, 0, 7, 8]]), // ▉ 左 7/8
    solid(&[[0, 0, 6, 8]]), // ▊ 左 3/4
    solid(&[[0, 0, 5, 8]]), // ▋ 左 5/8
    solid(&[[0, 0, 4, 8]]), // ▌ 左半
    solid(&[[0, 0, 3, 8]]), // ▍ 左 3/8
    solid(&[[0, 0, 2, 8]]), // ▎ 左 1/4
    solid(&[[0, 0, 1, 8]]), // ▏ 左 1/8
    solid(&[[4, 0, 8, 8]]), // ▐ 右半
    shade(0.25),            // ░
    shade(0.5),             // ▒
    shade(0.75),            // ▓
    solid(&[[0, 0, 8, 1]]), // ▔ 上 1/8
    solid(&[[7, 0, 8, 8]]), // ▕ 右 1/8
    solid(&[[0, 4, 4, 8]]), // ▖ 左下
    solid(&[[4, 4, 8, 8]]), // ▗ 右下
    solid(&[[0, 0, 4, 4]]), // ▘ 左上
    solid(&[[0, 0, 4, 8], [4, 4, 8, 8]]), // ▙ 左上 + 左下 + 右下
    solid(&[[0, 0, 4, 4], [4, 4, 8, 8]]), // ▚ 左上 + 右下
    solid(&[[0, 0, 8, 4], [0, 4, 4, 8]]), // ▛ 左上 + 右上 + 左下
    solid(&[[0, 0, 8, 4], [4, 4, 8, 8]]), // ▜ 左上 + 右上 + 右下
    solid(&[[4, 0, 8, 4]]), // ▝ 右上
    solid(&[[4, 0, 8, 4], [0, 4, 4, 8]]), // ▞ 右上 + 左下
    solid(&[[4, 0, 8, 4], [0, 4, 8, 8]]), // ▟ 右上 + 左下 + 右下
];

/// 是方块字符就给出怎么画，别的字符交给字体。
pub fn block(ch: char) -> Option<Block> {
    let index = (ch as u32).checked_sub(0x2580)?;
    BLOCKS.get(index as usize).copied()
}

impl Block {
    /// 在 `cell` 这一格里要涂的矩形。分界（半格、几分之几格）对齐到设备像素；`cell` 本身的边原样用，
    /// 所以和相邻格子、格子底色的边一模一样。
    pub fn bounds(self, cell: Bounds<Pixels>, scale: f32) -> impl Iterator<Item = Bounds<Pixels>> {
        let edge = move |start: Pixels, length: Pixels, eighths: u8| match eighths {
            0 => start,
            8 => start + length,
            _ => start + px((f32::from(length) * f32::from(eighths) / 8. * scale).round() / scale),
        };
        self.rects.iter().map(move |&[left, top, right, bottom]| {
            Bounds::from_corners(
                point(edge(cell.origin.x, cell.size.width, left), edge(cell.origin.y, cell.size.height, top)),
                point(edge(cell.origin.x, cell.size.width, right), edge(cell.origin.y, cell.size.height, bottom)),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use gpui::size;

    use super::*;

    fn cell() -> Bounds<Pixels> {
        // 起点不在整数上（网格左上角常是半个像素），宽高同 13pt 的格子在 2 倍屏上
        Bounds::new(point(px(10.5), px(20.)), size(px(8.), px(17.)))
    }

    fn covers(ch: char, x: f32, y: f32) -> bool {
        let cell = cell();
        let at = point(cell.origin.x + cell.size.width * x, cell.origin.y + cell.size.height * y);
        block(ch).unwrap().bounds(cell, 2.).any(|b| b.contains(&at))
    }

    #[test]
    fn exactly_the_block_elements_are_drawn_by_hand() {
        for ch in '\u{2580}'..='\u{259F}' {
            assert!(block(ch).is_some(), "U+{:04X}", ch as u32);
        }
        for ch in ['\u{257F}', '\u{25A0}', '─', '│', 'a', ' ', '中'] {
            assert!(block(ch).is_none(), "U+{:04X}", ch as u32);
        }
    }

    /// █ 正好铺满格子：两个 █ 挨着就没有缝（Claude Code 标志的竖线）。
    #[test]
    fn a_full_block_fills_its_cell_exactly() {
        let rects: Vec<_> = block('█').unwrap().bounds(cell(), 2.).collect();
        assert_eq!(rects, vec![cell()]);
        // 左半 + 右半 = 整格，中间的分界是同一条
        let left = block('▌').unwrap().bounds(cell(), 2.).next().unwrap();
        let right = block('▐').unwrap().bounds(cell(), 2.).next().unwrap();
        assert_eq!((left.origin.x, left.right()), (cell().origin.x, right.origin.x));
        assert_eq!(right.right(), cell().right());
    }

    /// 四分之一块按 Unicode 的定义涂对了哪几块（第几位：左上、右上、左下、右下）。
    #[test]
    fn quadrants_match_their_unicode_names() {
        let quadrants = [
            ('▀', 0b1100),
            ('▄', 0b0011),
            ('▌', 0b1010),
            ('▐', 0b0101),
            ('█', 0b1111),
            ('▖', 0b0010),
            ('▗', 0b0001),
            ('▘', 0b1000),
            ('▙', 0b1011),
            ('▚', 0b1001),
            ('▛', 0b1110),
            ('▜', 0b1101),
            ('▝', 0b0100),
            ('▞', 0b0110),
            ('▟', 0b0111),
        ];
        for (ch, mask) in quadrants {
            let got = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                .iter()
                .fold(0, |bits, &(x, y)| bits << 1 | u8::from(covers(ch, x, y)));
            assert_eq!(got, mask, "{ch}");
        }
    }

    /// ▁…▇ 从下往上 n/8 格高，▉…▏ 从左往右 (8-n)/8 格宽，▔ ▕ 是上 / 右 1/8。
    #[test]
    fn eighths_grow_from_the_right_side() {
        for n in 1..=7u8 {
            let lower = char::from_u32(0x2580 + u32::from(n)).unwrap();
            assert!(covers(lower, 0.5, 1. - (f32::from(n) - 0.5) / 8.), "{lower} 该涂到 {n}/8");
            assert!(!covers(lower, 0.5, 1. - (f32::from(n) + 0.5) / 8.), "{lower} 不该超过 {n}/8");
            let left = char::from_u32(0x2590 - u32::from(n)).unwrap();
            assert!(covers(left, (f32::from(n) - 0.5) / 8., 0.5), "{left} 该宽 {n}/8");
            assert!(!covers(left, (f32::from(n) + 0.5) / 8., 0.5), "{left} 不该超过 {n}/8");
        }
        assert!(covers('▔', 0.5, 0.05) && !covers('▔', 0.5, 0.2));
        assert!(covers('▕', 0.95, 0.5) && !covers('▕', 0.8, 0.5));
    }

    /// 分界对齐到设备像素：格子高 17pt（2 倍屏 34 像素），下 3/8 的上边在 21.25 像素处，取整到 21。
    #[test]
    fn inner_edges_land_on_device_pixels() {
        for ch in '\u{2580}'..='\u{259F}' {
            for b in block(ch).unwrap().bounds(cell(), 2.) {
                for v in [b.origin.x - cell().origin.x, b.origin.y - cell().origin.y, b.right() - cell().origin.x, b.bottom() - cell().origin.y] {
                    let device = f32::from(v) * 2.;
                    assert_eq!(device, device.round(), "{ch}: {v:?}");
                }
            }
        }
        assert_eq!(block('▃').unwrap().bounds(cell(), 2.).next().unwrap().origin.y, cell().origin.y + px(21. / 2.));
    }

    #[test]
    fn shades_fill_the_cell_with_alpha() {
        for (ch, alpha) in [('░', 0.25), ('▒', 0.5), ('▓', 0.75)] {
            let block = block(ch).unwrap();
            assert_eq!(block.alpha, alpha);
            assert_eq!(block.bounds(cell(), 2.).collect::<Vec<_>>(), vec![cell()]);
        }
        assert_eq!(block('█').unwrap().alpha, 1.);
    }
}

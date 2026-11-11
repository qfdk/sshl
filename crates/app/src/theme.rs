//! 设计稿 B「纸页」：浅灰窗口底（shell）上放白色卡片，主色为蓝（色相 251°，加深到白字可读 4.66:1）。
//! 数值由旧界面 index.css 的 oklch 换算成 sRGB。

use gpui::{Hsla, rgb};

pub fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn alpha(hex: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(hex).into();
    c.a = a;
    c
}

/// 界面颜色（亮色）。
pub mod ui {
    pub const SHELL: u32 = 0xf3f4f6;
    pub const CARD: u32 = 0xffffff;
    pub const FOREGROUND: u32 = 0x020618;
    pub const PRIMARY: u32 = 0x2777c3;
    pub const PRIMARY_FOREGROUND: u32 = 0xf9fafb;
    pub const SECONDARY: u32 = 0xe6e8ec;
    pub const MUTED: u32 = 0xf1f5f9;
    pub const MUTED_FOREGROUND: u32 = 0x5b6270;
    pub const ACCENT: u32 = 0xe5f0fd;
    pub const ACCENT_FOREGROUND: u32 = 0x1d4975;
    pub const DESTRUCTIVE: u32 = 0xe7000b;
    pub const SUCCESS: u32 = 0x11b981;
    pub const WARNING: u32 = 0xd97706;
    /// 成功 / 警告色做文字时用深一号的（原色在白底上对比度不够）。
    pub const SUCCESS_FOREGROUND: u32 = 0x047857;
    pub const WARNING_FOREGROUND: u32 = 0xb45309;
    /// 「系统」页区分指标用：内存紫、磁盘青绿。
    pub const VIOLET: u32 = 0x7c3aed;
    pub const TEAL: u32 = 0x0f766e;
    pub const BORDER: u32 = 0xe2e8f0;
    pub const RING: u32 = 0x5695d7;
}

/// 终端配色：与用户 Ghostty 配置（Carbonfox 改）对齐。
pub mod term {
    use crate::terminal::frame::Rgb;

    pub const BACKGROUND: Rgb = Rgb::hex(0x1d1d1d);
    pub const FOREGROUND: Rgb = Rgb::hex(0xf6f825);
    pub const CURSOR: Rgb = Rgb::hex(0xf2f4f8);
    pub const SELECTION_BG: Rgb = Rgb::hex(0x2a2a2a);
    pub const SELECTION_FG: Rgb = Rgb::hex(0xf2f4f8);
    /// 搜索（⌘F）高亮：匹配是淡黄底黑字，当前选中的那个是橙底（同 Ghostty 默认）。
    pub const SEARCH_BG: Rgb = Rgb::hex(0xffe082);
    pub const SEARCH_SELECTED_BG: Rgb = Rgb::hex(0xf2a57e);
    pub const SEARCH_FG: Rgb = Rgb::hex(0x000000);
    pub const ANSI: [Rgb; 16] = [
        Rgb::hex(0x282828),
        Rgb::hex(0xee5396),
        Rgb::hex(0x00ff74),
        Rgb::hex(0x08bdba),
        Rgb::hex(0x78a9ff),
        Rgb::hex(0xbe95ff),
        Rgb::hex(0x33b1ff),
        Rgb::hex(0xdfdfe0),
        Rgb::hex(0x484848),
        Rgb::hex(0xf16da6),
        Rgb::hex(0x46c880),
        Rgb::hex(0x2dc7c4),
        Rgb::hex(0x8cb6ff),
        Rgb::hex(0xc8a5ff),
        Rgb::hex(0x52bdff),
        Rgb::hex(0xe4e4e5),
    ];
}

/// 终端卡片四边的内边距（逻辑像素）。
pub const TERMINAL_PADDING: f32 = 4.0;

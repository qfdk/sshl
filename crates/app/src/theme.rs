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

/// gpui-base 行为层用到的主题色。输入框的选区取强调色的 40%：它默认的强调色是近白的浅灰，
/// 画在白底输入框上看不出选中了什么，换成主色蓝。
pub fn base_tokens() -> gpui_base::SemanticThemeTokens {
    let mut tokens = gpui_base::SemanticThemeTokens::default();
    tokens.colors.accent = color(ui::PRIMARY);
    tokens
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

/// 终端配色：内置几套，设置里选（`Settings::terminal_theme` 存 id）。默认是对齐用户 Ghostty 配置的 Carbonfox 改版。
/// 终端线程拿的是 `&'static Theme`，换配色时只传一个引用（`terminal::engine::Input::Theme`）。
pub mod term {
    use gpui::Hsla;

    use crate::terminal::frame::Rgb;

    /// 搜索（⌘F）高亮：匹配是淡黄底黑字，当前选中的那个是橙底（同 Ghostty 默认）。不跟主题变：在哪套配色上都看得清。
    pub const SEARCH_BG: Rgb = Rgb::hex(0xffe082);
    pub const SEARCH_SELECTED_BG: Rgb = Rgb::hex(0xf2a57e);
    pub const SEARCH_FG: Rgb = Rgb::hex(0x000000);

    /// 一套终端配色。
    #[derive(Debug, PartialEq, Eq)]
    pub struct Theme {
        /// 存进设置的名字。定了就别改：改了的话存着旧 id 的设置会退回默认。
        pub id: &'static str,
        /// 设置里显示的名字（也是无障碍标签）。
        pub name: &'static str,
        pub background: Rgb,
        pub foreground: Rgb,
        pub cursor: Rgb,
        pub selection_bg: Rgb,
        /// 选中的字；None = 保持字原来的颜色。
        pub selection_fg: Option<Rgb>,
        /// 0–7 普通色，8–15 亮色。
        pub ansi: [Rgb; 16],
    }

    impl Theme {
        /// 浅底（底色上黑字比白字清楚）：空页面的提示字、设置里的预览按它挑颜色。
        pub fn is_light(&self) -> bool {
            luminance(self.background) > 0.18
        }
    }

    pub const DEFAULT_ID: &str = "carbonfox";

    /// 按 id 找；不认识的（手改的设置、以后删掉的主题）用默认的，存的 id 不动。
    pub fn find(id: &str) -> &'static Theme {
        THEMES.iter().find(|theme| theme.id == id).unwrap_or(&THEMES[0])
    }

    /// 终端颜色换成界面画图用的。
    pub fn hsla(c: Rgb) -> Hsla {
        super::color(c.to_u32())
    }

    /// WCAG 相对亮度（0 黑 – 1 白）。
    pub fn luminance(c: Rgb) -> f32 {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    const fn palette(hex: [u32; 16]) -> [Rgb; 16] {
        let mut out = [Rgb::hex(0); 16];
        let mut i = 0;
        while i < 16 {
            out[i] = Rgb::hex(hex[i]);
            i += 1;
        }
        out
    }

    /// 内置配色：默认的排第一，然后深色、浅色。颜色都取自各主题的官方配置（每套注明出处），不凭记忆写。
    pub static THEMES: &[Theme] = &[
        // 用户 Ghostty 配置里的 Carbonfox 改版（原来写死的那套），一个颜色都不能变。
        Theme {
            id: DEFAULT_ID,
            name: "Carbonfox",
            background: Rgb::hex(0x1d1d1d),
            foreground: Rgb::hex(0xf6f825),
            cursor: Rgb::hex(0xf2f4f8),
            selection_bg: Rgb::hex(0x2a2a2a),
            selection_fg: Some(Rgb::hex(0xf2f4f8)),
            ansi: palette([
                0x282828, 0xee5396, 0x00ff74, 0x08bdba, 0x78a9ff, 0xbe95ff, 0x33b1ff, 0xdfdfe0,
                0x484848, 0xf16da6, 0x46c880, 0x2dc7c4, 0x8cb6ff, 0xc8a5ff, 0x52bdff, 0xe4e4e5,
            ]),
        },
        // folke/tokyonight.nvim 的 extras/ghostty/tokyonight_night。
        Theme {
            id: "tokyo-night",
            name: "Tokyo Night",
            background: Rgb::hex(0x1a1b26),
            foreground: Rgb::hex(0xc0caf5),
            cursor: Rgb::hex(0xc0caf5),
            selection_bg: Rgb::hex(0x283457),
            selection_fg: Some(Rgb::hex(0xc0caf5)),
            ansi: palette([
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                0x414868, 0xff899d, 0x9fe044, 0xfaba4a, 0x8db0ff, 0xc7a9ff, 0xa4daff, 0xc0caf5,
            ]),
        },
        // 官方 dracula/ghostty 仓库的 dracula。
        Theme {
            id: "dracula",
            name: "Dracula",
            background: Rgb::hex(0x282a36),
            foreground: Rgb::hex(0xf8f8f2),
            cursor: Rgb::hex(0xf8f8f2),
            selection_bg: Rgb::hex(0x44475a),
            selection_fg: Some(Rgb::hex(0xf8f8f2)),
            ansi: palette([
                0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
            ]),
        },
        // 官方 catppuccin/ghostty 仓库的 themes/catppuccin-mocha.conf。
        Theme {
            id: "catppuccin-mocha",
            name: "Catppuccin Mocha",
            background: Rgb::hex(0x1e1e2e),
            foreground: Rgb::hex(0xcdd6f4),
            cursor: Rgb::hex(0xf5e0dc),
            selection_bg: Rgb::hex(0x353749),
            selection_fg: Some(Rgb::hex(0xcdd6f4)),
            ansi: palette([
                0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
                0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
            ]),
        },
        // Ghostty 自带的 Gruvbox Dark（iTerm2-Color-Schemes 的 ghostty/Gruvbox Dark）；
        // 底色、前景和 16 色与官方 morhetz/gruvbox-contrib 的 xresources/gruvbox-dark.xresources 一致。
        Theme {
            id: "gruvbox-dark",
            name: "Gruvbox Dark",
            background: Rgb::hex(0x282828),
            foreground: Rgb::hex(0xebdbb2),
            cursor: Rgb::hex(0xebdbb2),
            selection_bg: Rgb::hex(0x665c54),
            selection_fg: Some(Rgb::hex(0xebdbb2)),
            ansi: palette([
                0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
                0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
            ]),
        },
        // 官方 nordtheme/alacritty 的 src/nord.yaml（选中的字保持原色：那里写的是 CellForeground）。
        Theme {
            id: "nord",
            name: "Nord",
            background: Rgb::hex(0x2e3440),
            foreground: Rgb::hex(0xd8dee9),
            cursor: Rgb::hex(0xd8dee9),
            selection_bg: Rgb::hex(0x4c566a),
            selection_fg: None,
            ansi: palette([
                0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
            ]),
        },
        // Ghostty 自带的 Atom One Dark（iTerm2-Color-Schemes 的 ghostty/Atom One Dark）。
        Theme {
            id: "one-dark",
            name: "One Dark",
            background: Rgb::hex(0x21252b),
            foreground: Rgb::hex(0xabb2bf),
            cursor: Rgb::hex(0xabb2bf),
            selection_bg: Rgb::hex(0x323844),
            selection_fg: Some(Rgb::hex(0xabb2bf)),
            ansi: palette([
                0x21252b, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                0x767676, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
            ]),
        },
        // Ghostty 自带的 iTerm2 Solarized Dark（iTerm2-Color-Schemes）：除 8 号外都是官方 altercation/solarized 色表里的值；
        // 官方 8 号（亮黑）是 base03，和底色一样，zsh 自动补全提示之类的灰字会看不见，iTerm2 把它提亮成 #335e69。
        Theme {
            id: "solarized-dark",
            name: "Solarized Dark",
            background: Rgb::hex(0x002b36),
            foreground: Rgb::hex(0x839496),
            cursor: Rgb::hex(0x839496),
            selection_bg: Rgb::hex(0x073642),
            selection_fg: Some(Rgb::hex(0x93a1a1)),
            ansi: palette([
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x335e69, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
            ]),
        },
        // Ghostty 自带的 iTerm2 Solarized Light（iTerm2-Color-Schemes），只改了前景和光标：官方的 base00（#657b83）
        // 对底色只有 4.1:1，不到 4.5，换成官方色表里同属正文、浅色模式下用于强调的 base01（#586e75，5.0:1）。
        Theme {
            id: "solarized-light",
            name: "Solarized Light",
            background: Rgb::hex(0xfdf6e3),
            foreground: Rgb::hex(0x586e75),
            cursor: Rgb::hex(0x586e75),
            selection_bg: Rgb::hex(0xeee8d5),
            selection_fg: Some(Rgb::hex(0x586e75)),
            ansi: palette([
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xbbb5a2,
                0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
            ]),
        },
        // GitHub 官方 VS Code 主题（primer/github-vscode-theme 6.3.5 的 themes/light-default.json）的底色、前景、光标和 16 色；
        // 它没定选区，选区取 Ghostty 自带的 GitHub Light Default（iTerm2-Color-Schemes）。
        Theme {
            id: "github-light",
            name: "GitHub Light",
            background: Rgb::hex(0xffffff),
            foreground: Rgb::hex(0x1f2328),
            cursor: Rgb::hex(0x0969da),
            selection_bg: Rgb::hex(0x1f2328),
            selection_fg: Some(Rgb::hex(0xffffff)),
            ansi: palette([
                0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
                0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
            ]),
        },
        // 官方 catppuccin/ghostty 仓库的 themes/catppuccin-latte.conf，只改了光标：官方的 Rosewater（#dc8a78）
        // 对底色只有 2.3:1，默认光标又是 1px 的竖线，在浅底上不好找，换成同一色板的 Text（#4c4f69）。
        Theme {
            id: "catppuccin-latte",
            name: "Catppuccin Latte",
            background: Rgb::hex(0xeff1f5),
            foreground: Rgb::hex(0x4c4f69),
            cursor: Rgb::hex(0x4c4f69),
            selection_bg: Rgb::hex(0xd8dae1),
            selection_fg: Some(Rgb::hex(0x4c4f69)),
            ansi: palette([
                0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
                0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
            ]),
        },
    ];
}

/// 终端卡片四边的内边距（逻辑像素）。
pub const TERMINAL_PADDING: f32 = 4.0;

#[cfg(test)]
mod tests {
    use gpui::{Hsla, Rgba};
    use gpui_base::input::InputEditorStyle;

    use super::*;
    use crate::terminal::frame::Rgb;

    fn luminance(c: Rgba) -> f32 {
        let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn over(top: Hsla, bottom: Hsla) -> Rgba {
        let (t, b) = (Rgba::from(top), Rgba::from(bottom));
        let mix = |t: f32, b: f32| t * top.a + b * (1. - top.a);
        Rgba { r: mix(t.r, b.r), g: mix(t.g, b.g), b: mix(t.b, b.b), a: 1. }
    }

    #[test]
    fn input_selection_is_visible_on_a_white_field() {
        let selection = InputEditorStyle::default().resolved(&base_tokens()).selection;
        let field = color(ui::CARD);
        let contrast = (luminance(field.into()) + 0.05) / (luminance(over(selection, field)) + 0.05);
        assert!(contrast > 1.4, "选区和输入框底色分不出来：对比度 {contrast}");
        // 选区下的字还要看得清
        let text = (luminance(over(selection, field)) + 0.05) / (luminance(color(ui::FOREGROUND).into()) + 0.05);
        assert!(text > 7., "选区里的字不清楚：对比度 {text}");
    }

    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let (a, b) = (term::luminance(a), term::luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn terminal_theme_ids_and_names_are_unique() {
        let ids: std::collections::HashSet<&str> = term::THEMES.iter().map(|t| t.id).collect();
        let names: std::collections::HashSet<&str> = term::THEMES.iter().map(|t| t.name).collect();
        assert_eq!(ids.len(), term::THEMES.len(), "id 重了");
        assert_eq!(names.len(), term::THEMES.len(), "名字重了");
        assert_eq!(term::THEMES[0].id, term::DEFAULT_ID, "默认的排第一");
    }

    /// 默认配色就是原来写死的那套（用户 Ghostty 配置里的 Carbonfox 改版）：值抄自改动前的常量，一个都不能变。
    #[test]
    fn the_default_terminal_theme_keeps_the_original_carbonfox_colors() {
        let theme = term::find(term::DEFAULT_ID);
        assert_eq!((theme.id, theme.name), ("carbonfox", "Carbonfox"));
        assert_eq!(theme.background, Rgb::hex(0x1d1d1d));
        assert_eq!(theme.foreground, Rgb::hex(0xf6f825));
        assert_eq!(theme.cursor, Rgb::hex(0xf2f4f8));
        assert_eq!(theme.selection_bg, Rgb::hex(0x2a2a2a));
        assert_eq!(theme.selection_fg, Some(Rgb::hex(0xf2f4f8)));
        let ansi = [
            0x282828, 0xee5396, 0x00ff74, 0x08bdba, 0x78a9ff, 0xbe95ff, 0x33b1ff, 0xdfdfe0, 0x484848, 0xf16da6, 0x46c880,
            0x2dc7c4, 0x8cb6ff, 0xc8a5ff, 0x52bdff, 0xe4e4e5,
        ];
        assert_eq!(theme.ansi, ansi.map(Rgb::hex));
    }

    #[test]
    fn unknown_terminal_themes_fall_back_to_the_default() {
        assert_eq!(term::find("dracula").name, "Dracula");
        assert_eq!(term::find("no-such-theme").id, term::DEFAULT_ID);
        assert_eq!(term::find("").id, term::DEFAULT_ID);
    }

    /// 每套的字都看得清（正文 4.5:1），光标也看得见（非文字元素 3:1；默认光标只有 1px 宽）。
    #[test]
    fn every_terminal_theme_has_readable_text_and_a_visible_cursor() {
        for theme in term::THEMES {
            let text = contrast(theme.foreground, theme.background);
            assert!(text >= 4.5, "{}：前景对背景只有 {text:.2}:1", theme.name);
            let cursor = contrast(theme.cursor, theme.background);
            assert!(cursor >= 3., "{}：光标对背景只有 {cursor:.2}:1", theme.name);
        }
    }

    #[test]
    fn light_terminal_themes_are_recognized() {
        let light: Vec<&str> = term::THEMES.iter().filter(|t| t.is_light()).map(|t| t.id).collect();
        assert_eq!(light, ["solarized-light", "github-light", "catppuccin-latte"]);
    }
}

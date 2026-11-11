//! 设计稿 B「纸页」：浅灰窗口底（shell）上放白色卡片，主色为蓝（色相 251°，加深到白字可读 4.66:1）。
//! 亮色数值由旧界面 index.css 的 oklch 换算成 sRGB；深色是同一套角色的另一组值（`ui::Token` 一个角色两个值）。

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{Hsla, WindowAppearance, rgb};
use serde::{Deserialize, Serialize};

/// 界面现在是不是深色。只在界面线程改（`set_dark`），画的时候读。
static DARK: AtomicBool = AtomicBool::new(false);

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// 返回有没有变。变了之后调用方要重画窗口、把 gpui-base 的 token 换掉（`base_tokens`）。
pub fn set_dark(dark: bool) -> bool {
    DARK.swap(dark, Ordering::Relaxed) != dark
}

/// 设置里的「外观」。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// 跟随系统的浅色 / 深色。
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn is_dark(self, system: WindowAppearance) -> bool {
        match self {
            Appearance::System => matches!(system, WindowAppearance::Dark | WindowAppearance::VibrantDark),
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }
}

/// 能当颜色用的：`ui` 里的角色（跟着外观变），或者写死的 0xRRGGBB（不变）。
pub trait Tone {
    fn hex(self) -> u32;
}

impl Tone for u32 {
    fn hex(self) -> u32 {
        self
    }
}

impl Tone for ui::Token {
    fn hex(self) -> u32 {
        self.resolve(is_dark())
    }
}

pub fn color(tone: impl Tone) -> Hsla {
    rgb(tone.hex()).into()
}

pub fn alpha(tone: impl Tone, a: f32) -> Hsla {
    let mut c: Hsla = rgb(tone.hex()).into();
    c.a = a;
    c
}

/// gpui-base 行为层用到的主题色：输入框的字、占位字、光标取这里的前景色。输入框的选区取强调色的 40%：
/// 它默认的强调色是近白的浅灰，画在白底输入框上看不出选中了什么，换成主色蓝。
pub fn base_tokens(dark: bool) -> gpui_base::SemanticThemeTokens {
    let pick = |token: ui::Token| -> Hsla { rgb(token.resolve(dark)).into() };
    let mut tokens = gpui_base::SemanticThemeTokens::default();
    if dark {
        tokens.colors = gpui_base::ColorTokens::dark();
        tokens.colors.background = pick(ui::SHELL);
        tokens.colors.surface = pick(ui::CARD);
        tokens.colors.foreground = pick(ui::FOREGROUND);
        tokens.colors.surface_foreground = pick(ui::FOREGROUND);
        tokens.colors.muted_foreground = pick(ui::MUTED_FOREGROUND);
        tokens.colors.border = pick(ui::BORDER);
    }
    tokens.colors.accent = pick(ui::PRIMARY);
    tokens
}

/// 界面颜色：每个角色一对值（亮色、深色）。
pub mod ui {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Token {
        light: u32,
        dark: u32,
    }

    impl Token {
        pub const fn resolve(self, dark: bool) -> u32 {
            if dark { self.dark } else { self.light }
        }
    }

    const fn token(light: u32, dark: u32) -> Token {
        Token { light, dark }
    }

    /// 窗口底（侧栏、顶栏）。深色时比卡片更暗。
    pub const SHELL: Token = token(0xf3f4f6, 0x17181b);
    pub const CARD: Token = token(0xffffff, 0x222327);
    /// 槽里浮起来的那一块（分段控件的选中项）：亮色是白卡片，深色要比槽亮。
    pub const RAISED: Token = token(0xffffff, 0x41444c);
    pub const FOREGROUND: Token = token(0x020618, 0xe8eaee);
    /// 主色：既做按钮底（上面是白字），也做文字 / 图标。深色时提亮到在深卡片上看得清，白字还读得出。
    pub const PRIMARY: Token = token(0x2777c3, 0x3d8bdb);
    pub const PRIMARY_FOREGROUND: Token = token(0xf9fafb, 0xffffff);
    pub const SECONDARY: Token = token(0xe6e8ec, 0x2d2f35);
    pub const MUTED: Token = token(0xf1f5f9, 0x2a2c31);
    pub const MUTED_FOREGROUND: Token = token(0x5b6270, 0x9ba1ad);
    pub const ACCENT: Token = token(0xe5f0fd, 0x213b58);
    pub const ACCENT_FOREGROUND: Token = token(0x1d4975, 0xbddafa);
    pub const DESTRUCTIVE: Token = token(0xe7000b, 0xf0535b);
    pub const SUCCESS: Token = token(0x11b981, 0x11b981);
    pub const WARNING: Token = token(0xd97706, 0xe08a1a);
    /// 成功 / 警告色做文字时：亮色用深一号的（原色在白底上对比度不够），深色用亮一号的。
    pub const SUCCESS_FOREGROUND: Token = token(0x047857, 0x34d399);
    pub const WARNING_FOREGROUND: Token = token(0xb45309, 0xfbbf24);
    /// 「系统」页区分指标用：内存紫、磁盘青绿。
    pub const VIOLET: Token = token(0x7c3aed, 0xa78bfa);
    pub const TEAL: Token = token(0x0f766e, 0x2dd4bf);
    pub const BORDER: Token = token(0xe2e8f0, 0x34373d);
    /// 没勾的复选框的边：比普通边框深一号，在卡片上认得出是个框。
    pub const CHECK_BORDER: Token = token(0xcbd5e1, 0x555963);
    /// 描边按钮（取消、还原默认）：亮色是白底细边；深色时底和边都比卡片亮一号，不然在深底上只剩一块更暗的影子，看着比旁边的主按钮小。
    pub const CONTROL: Token = token(0xffffff, 0x33363d);
    pub const CONTROL_BORDER: Token = token(0xe2e8f0, 0x4a4e57);
    pub const RING: Token = token(0x5695d7, 0x5695d7);
    /// 气泡提示：亮色是深底浅字，深色是比卡片亮一号的底。
    pub const TOOLTIP: Token = token(0x020618, 0x41444c);
    pub const TOOLTIP_FOREGROUND: Token = token(0xf9fafb, 0xf4f5f7);
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
        let selection = InputEditorStyle::default().resolved(&base_tokens(false)).selection;
        let field = color(ui::CARD);
        let contrast = (luminance(field.into()) + 0.05) / (luminance(over(selection, field)) + 0.05);
        assert!(contrast > 1.4, "选区和输入框底色分不出来：对比度 {contrast}");
        // 选区下的字还要看得清
        let text = (luminance(over(selection, field)) + 0.05) / (luminance(color(ui::FOREGROUND).into()) + 0.05);
        assert!(text > 7., "选区里的字不清楚：对比度 {text}");
    }

    fn ui_contrast(a: ui::Token, b: ui::Token, dark: bool) -> f32 {
        let l = |t: ui::Token| luminance(gpui::rgb(t.resolve(dark)));
        let (a, b) = (l(a), l(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn the_appearance_setting_decides_light_or_dark() {
        use gpui::WindowAppearance::{Dark, Light, VibrantDark, VibrantLight};
        assert!(!Appearance::System.is_dark(Light) && !Appearance::System.is_dark(VibrantLight));
        assert!(Appearance::System.is_dark(Dark) && Appearance::System.is_dark(VibrantDark), "跟随系统");
        assert!(!Appearance::Light.is_dark(Dark), "定了浅色就不看系统");
        assert!(Appearance::Dark.is_dark(Light), "定了深色就不看系统");
    }

    /// 两种外观下字都看得清：正文 7:1、次要文字 4.5:1（卡片和窗口底上都要），主色 / 红色做字和按钮上的白字 3:1 以上。
    #[test]
    fn both_appearances_keep_text_readable() {
        for dark in [false, true] {
            let name = if dark { "深色" } else { "浅色" };
            for surface in [ui::CARD, ui::SHELL, ui::MUTED] {
                let text = ui_contrast(ui::FOREGROUND, surface, dark);
                assert!(text >= 7., "{name}：正文只有 {text:.2}:1");
                let muted = ui_contrast(ui::MUTED_FOREGROUND, surface, dark);
                assert!(muted >= 4.5, "{name}：次要文字只有 {muted:.2}:1");
            }
            for accent in [ui::PRIMARY, ui::DESTRUCTIVE] {
                let on_card = ui_contrast(accent, ui::CARD, dark);
                assert!(on_card >= 3., "{name}：强调色在卡片上只有 {on_card:.2}:1");
                let label = ui_contrast(ui::PRIMARY_FOREGROUND, accent, dark);
                assert!(label >= 3., "{name}：按钮上的字只有 {label:.2}:1");
            }
            let hover = ui_contrast(ui::ACCENT_FOREGROUND, ui::ACCENT, dark);
            assert!(hover >= 4.5, "{name}：悬停行上的字只有 {hover:.2}:1");
            let tip = ui_contrast(ui::TOOLTIP_FOREGROUND, ui::TOOLTIP, dark);
            assert!(tip >= 7., "{name}：气泡提示的字只有 {tip:.2}:1");
        }
    }

    /// 深色时层次反过来：窗口底最暗，卡片亮一点，浮起来的选中项和描边按钮再亮一点。
    #[test]
    fn dark_surfaces_get_lighter_as_they_rise() {
        let l = |t: ui::Token| luminance(gpui::rgb(t.resolve(true)));
        assert!(l(ui::SHELL) < l(ui::CARD));
        assert!(l(ui::CARD) < l(ui::CONTROL) && l(ui::CONTROL) < l(ui::CONTROL_BORDER));
        assert!(l(ui::SECONDARY) < l(ui::RAISED), "分段控件的选中项要比槽亮");
        // 输入框里的字跟着外观走
        let tokens = base_tokens(true);
        assert_eq!(tokens.colors.foreground, color(ui::FOREGROUND.resolve(true)));
        assert_eq!(base_tokens(false).colors.accent, color(ui::PRIMARY.resolve(false)));
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

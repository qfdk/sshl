//! 打进二进制的资源：Lucide 图标（ISC，线宽 2.5）和应用图标。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon {
            $($variant),*
        }

        impl Icon {
            pub fn path(self) -> SharedString {
                match self {
                    $(Icon::$variant => concat!("icons/", $file, ".svg").into()),*
                }
            }
        }

        fn icon_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("icons/", $file, ".svg") => Some(include_bytes!(concat!("../../../assets/icons/lucide/", $file, ".svg"))),)*
                _ => None,
            }
        }
    };
}

icons! {
    ArrowDownUp => "arrow-down-up",
    ArrowUp => "arrow-up",
    Bookmark => "bookmark",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    Clock => "clock",
    CodeXml => "code-xml",
    Download => "download",
    FileText => "file-text",
    Folder => "folder",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    Gauge => "gauge",
    GripVertical => "grip-vertical",
    HardDrive => "hard-drive",
    House => "house",
    KeyRound => "key-round",
    LoaderCircle => "loader-circle",
    Lock => "lock",
    MemoryStick => "memory-stick",
    Pencil => "pencil",
    Plus => "plus",
    Power => "power",
    RefreshCw => "refresh-cw",
    Search => "search",
    Server => "server",
    Settings => "settings",
    Trash => "trash-2",
    Type => "type",
    Upload => "upload",
    UserRound => "user-round",
    X => "x",
    Zap => "zap",
}

pub const APP_ICON: &str = "icon.png";
/// 侧栏标志的显示尺寸（pt）。图片按 2 倍屏事先缩好（见下面的测试），换图标后从根目录 icon.png 重新生成：
/// `magick icon.png -colorspace RGB -filter Box -resize 60x60 -colorspace sRGB -strip assets/sidebar-logo.png`
/// （线性光下按面积平均，不用 Lanczos：它在深色方块外缩出一圈浅色光环）
pub const APP_ICON_SIZE: f32 = 30.;
const APP_ICON_BYTES: &[u8] = include_bytes!("../../../assets/sidebar-logo.png");

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(APP_ICON_BYTES)));
        }
        Ok(icon_bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gpui 画图片是双线性采样、没有 mipmap：大图缩小时每个屏幕像素只取原图 4 个点，边缘发毛、细节断开。
    /// 侧栏标志要按 2 倍屏事先缩好，画出来正好一像素对一像素。
    #[test]
    fn app_icon_matches_retina_pixel_size() {
        assert_eq!(&APP_ICON_BYTES[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes(APP_ICON_BYTES[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(APP_ICON_BYTES[20..24].try_into().unwrap());
        let expected = (APP_ICON_SIZE * 2.) as u32;
        assert_eq!((width, height), (expected, expected));
    }
}

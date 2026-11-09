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
    ArrowRight => "arrow-right",
    ArrowUp => "arrow-up",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    Download => "download",
    FileText => "file-text",
    Folder => "folder",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    GripVertical => "grip-vertical",
    House => "house",
    KeyRound => "key-round",
    LoaderCircle => "loader-circle",
    Lock => "lock",
    Pencil => "pencil",
    Plus => "plus",
    Power => "power",
    RefreshCw => "refresh-cw",
    RotateCcw => "rotate-ccw",
    Save => "save",
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

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(include_bytes!("../../../assets/icon.png"))));
        }
        Ok(icon_bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

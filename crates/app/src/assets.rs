//! 打进二进制的资源：Lucide 图标（ISC，线宽 2.5）、服务端软件的标志（Simple Icons，CC0）和应用图标。

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
    ArrowDown => "arrow-down",
    ArrowDownUp => "arrow-down-up",
    ArrowUp => "arrow-up",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    Clock => "clock",
    CodeXml => "code-xml",
    Copy => "copy",
    Download => "download",
    FileText => "file-text",
    Folder => "folder",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    GripVertical => "grip-vertical",
    House => "house",
    Keyboard => "keyboard",
    KeyRound => "key-round",
    LoaderCircle => "loader-circle",
    Lock => "lock",
    PanelLeft => "panel-left",
    Pencil => "pencil",
    Plus => "plus",
    RefreshCw => "refresh-cw",
    RotateCcw => "rotate-ccw",
    Search => "search",
    Server => "server",
    Settings => "settings",
    SquareTerminal => "square-terminal",
    Moon => "moon",
    Star => "star",
    StarFilled => "star-filled",
    Trash => "trash-2",
    Type => "type",
    Upload => "upload",
    UserRound => "user-round",
    X => "x",
    Zap => "zap",
}

/// 服务端软件的标志（Simple Icons 16.36.0，CC0，单色路径 + 品牌标准色）：进程列表认出常见软件时画它（`brands.rs`）。
macro_rules! brands {
    ($($variant:ident => $file:literal, $hex:literal, $title:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Brand {
            $($variant),*
        }

        impl Brand {
            #[cfg(test)]
            pub const ALL: &[Brand] = &[$(Brand::$variant),*];

            pub fn path(self) -> SharedString {
                match self {
                    $(Brand::$variant => concat!("brands/", $file, ".svg").into()),*
                }
            }

            /// 品牌标准色（0xRRGGBB）。
            pub fn hex(self) -> u32 {
                match self {
                    $(Brand::$variant => $hex),*
                }
            }

            #[cfg(test)]
            pub fn title(self) -> &'static str {
                match self {
                    $(Brand::$variant => $title),*
                }
            }
        }

        fn brand_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("brands/", $file, ".svg") => Some(include_bytes!(concat!("../../../assets/icons/brands/", $file, ".svg"))),)*
                _ => None,
            }
        }
    };
}

brands! {
    AdGuard => "adguard", 0x68bc71, "AdGuard",
    Apache => "apache", 0xd22128, "Apache",
    Bun => "bun", 0x000000, "Bun",
    Caddy => "caddy", 0x1f88c0, "Caddy",
    Claude => "claude", 0xd97757, "Claude",
    ClickHouse => "clickhouse", 0xffcc01, "ClickHouse",
    Cloudflare => "cloudflare", 0xf38020, "Cloudflare",
    Consul => "consul", 0xf24c53, "Consul",
    Containerd => "containerd", 0x575757, "containerd",
    Deno => "deno", 0x000000, "Deno",
    Docker => "docker", 0x2496ed, "Docker",
    Dovecot => "dovecot", 0x54bcab, "Dovecot",
    Mosquitto => "eclipsemosquitto", 0x3c5280, "Eclipse Mosquitto",
    Envoy => "envoyproxy", 0xac6199, "Envoy Proxy",
    Etcd => "etcd", 0x419eda, "etcd",
    Gitea => "gitea", 0x609926, "Gitea",
    GitLab => "gitlab", 0xfc6d26, "GitLab",
    Grafana => "grafana", 0xf46800, "Grafana",
    Gunicorn => "gunicorn", 0x499848, "Gunicorn",
    HaProxy => "haproxy", 0x169bd6, "HAProxy",
    InfluxDb => "influxdb", 0x22adf6, "InfluxDB",
    Jellyfin => "jellyfin", 0x00a4dc, "Jellyfin",
    K3s => "k3s", 0xffc61c, "K3s",
    Kubernetes => "kubernetes", 0x326ce5, "Kubernetes",
    MariaDb => "mariadb", 0x003545, "MariaDB",
    Meilisearch => "meilisearch", 0xff5caa, "Meilisearch",
    MinIo => "minio", 0xc72e49, "MinIO",
    MongoDb => "mongodb", 0x47a248, "MongoDB",
    MySql => "mysql", 0x4479a1, "MySQL",
    Nats => "natsdotio", 0x27aae1, "NATS.io",
    Neovim => "neovim", 0x57a143, "Neovim",
    Nginx => "nginx", 0x009639, "NGINX",
    Node => "nodedotjs", 0x5fa04e, "Node.js",
    Nomad => "nomad", 0x00ca8e, "Nomad",
    Java => "openjdk", 0x000000, "OpenJDK",
    OpenVpn => "openvpn", 0xea7e20, "OpenVPN",
    Php => "php", 0x777bb4, "PHP",
    PiHole => "pihole", 0x96060c, "Pi-hole",
    Plex => "plex", 0xebaf00, "Plex",
    Pm2 => "pm2", 0x2b037a, "PM2",
    Podman => "podman", 0x892ca0, "Podman",
    Portainer => "portainer", 0x13bef9, "Portainer",
    Postgres => "postgresql", 0x4169e1, "PostgreSQL",
    Prometheus => "prometheus", 0xe6522c, "Prometheus",
    Python => "python", 0x3776ab, "Python",
    QBittorrent => "qbittorrent", 0x2f67ba, "qbittorrent",
    Redis => "redis", 0xff4438, "Redis",
    Ruby => "ruby", 0xcc342d, "Ruby",
    Syncthing => "syncthing", 0x0891d1, "Syncthing",
    Tailscale => "tailscale", 0x242424, "Tailscale",
    Tmux => "tmux", 0x1bb91f, "tmux",
    Traefik => "traefikproxy", 0x24a1c1, "Traefik Proxy",
    Transmission => "transmission", 0xd70008, "Transmission",
    Vault => "vault", 0xffec6e, "Vault",
    Vim => "vim", 0x019733, "Vim",
    WireGuard => "wireguard", 0x88171a, "WireGuard",
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
        Ok(icon_bytes(path).or_else(|| brand_bytes(path)).map(Cow::Borrowed))
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

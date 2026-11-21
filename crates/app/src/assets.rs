//! 打进二进制的资源：Lucide 图标（ISC，线宽 2.5）、服务端软件和系统的标志（Simple Icons，CC0；几个系统另有官方彩色版）和应用图标。

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
    HardDrive => "hard-drive",
    House => "house",
    Info => "info",
    Keyboard => "keyboard",
    KeyRound => "key-round",
    LoaderCircle => "loader-circle",
    Lock => "lock",
    MessageSquare => "message-square",
    PanelLeft => "panel-left",
    Pencil => "pencil",
    Plus => "plus",
    RefreshCw => "refresh-cw",
    RotateCcw => "rotate-ccw",
    Scale => "scale",
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

/// 服务端软件、AI 工具和系统的标志（单色路径 + 品牌标准色；Simple Icons 16.36.0，Redis 是 12.0.0，OpenAI 来自 gilbarbara/logos；
/// 大多是 CC0，个别另有许可，见 `assets/icons/brands/LICENSE`）：进程列表认出常见软件时画它，侧栏的连接画连上时认出的系统（`brands.rs`）。
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
    OnePanel => "1panel", 0x0854c1, "1Panel",
    AdGuard => "adguard", 0x68bc71, "AdGuard",
    AlibabaCloud => "alibabacloud", 0xff6a00, "Alibaba Cloud",
    AlmaLinux => "almalinux", 0x000000, "AlmaLinux",
    AlpineLinux => "alpinelinux", 0x0d597f, "Alpine Linux",
    Apache => "apache", 0xd22128, "Apache",
    Apple => "apple", 0x000000, "Apple",
    ArchLinux => "archlinux", 0x1793d1, "Arch Linux",
    Bun => "bun", 0x000000, "Bun",
    Caddy => "caddy", 0x1f88c0, "Caddy",
    CentOs => "centos", 0x262577, "CentOS",
    Claude => "claude", 0xd97757, "Claude",
    ClickHouse => "clickhouse", 0xffcc01, "ClickHouse",
    Cloudflare => "cloudflare", 0xf38020, "Cloudflare",
    Consul => "consul", 0xf24c53, "Consul",
    Containerd => "containerd", 0x575757, "containerd",
    Cursor => "cursor", 0x000000, "Cursor",
    Debian => "debian", 0xa81d33, "Debian",
    Deepin => "deepin", 0x007cff, "deepin",
    Deno => "deno", 0x000000, "Deno",
    Docker => "docker", 0x2496ed, "Docker",
    Dovecot => "dovecot", 0x54bcab, "Dovecot",
    Mosquitto => "eclipsemosquitto", 0x3c5280, "Eclipse Mosquitto",
    Elementary => "elementary", 0x64baff, "elementary",
    EndeavourOs => "endeavouros", 0x7f7fff, "EndeavourOS",
    Envoy => "envoyproxy", 0xac6199, "Envoy Proxy",
    Erlang => "erlang", 0xa90533, "Erlang",
    Etcd => "etcd", 0x419eda, "etcd",
    Fedora => "fedora", 0x51a2da, "Fedora",
    FreeBsd => "freebsd", 0xab2b28, "FreeBSD",
    Gentoo => "gentoo", 0x54487a, "Gentoo",
    Gitea => "gitea", 0x609926, "Gitea",
    GitHubActions => "githubactions", 0x2088ff, "GitHub Actions",
    Copilot => "githubcopilot", 0x000000, "GitHub Copilot",
    GitLab => "gitlab", 0xfc6d26, "GitLab",
    Gemini => "googlegemini", 0x8e75b2, "Google Gemini",
    Grafana => "grafana", 0xf46800, "Grafana",
    Gunicorn => "gunicorn", 0x499848, "Gunicorn",
    HaProxy => "haproxy", 0x169bd6, "HAProxy",
    HuggingFace => "huggingface", 0xffd21e, "Hugging Face",
    InfluxDb => "influxdb", 0x22adf6, "InfluxDB",
    Ipfs => "ipfs", 0x65c2cb, "IPFS",
    Jellyfin => "jellyfin", 0x00a4dc, "Jellyfin",
    Jupyter => "jupyter", 0xf37626, "Jupyter",
    K3s => "k3s", 0xffc61c, "K3s",
    KaliLinux => "kalilinux", 0x557c94, "Kali Linux",
    Kubernetes => "kubernetes", 0x326ce5, "Kubernetes",
    Linux => "linux", 0xfcc624, "Linux",
    LinuxMint => "linuxmint", 0x86be43, "Linux Mint",
    Manjaro => "manjaro", 0x35bfa4, "Manjaro",
    MariaDb => "mariadb", 0x003545, "MariaDB",
    Meilisearch => "meilisearch", 0xff5caa, "Meilisearch",
    MinIo => "minio", 0xc72e49, "MinIO",
    MlFlow => "mlflow", 0x0194e2, "MLflow",
    MongoDb => "mongodb", 0x47a248, "MongoDB",
    MySql => "mysql", 0x4479a1, "MySQL",
    N8n => "n8n", 0xea4b71, "n8n",
    Nats => "natsdotio", 0x27aae1, "NATS.io",
    Neovim => "neovim", 0x57a143, "Neovim",
    NextJs => "nextdotjs", 0x000000, "Next.js",
    Nginx => "nginx", 0x009639, "NGINX",
    NixOs => "nixos", 0x5277c3, "NixOS",
    Node => "nodedotjs", 0x5fa04e, "Node.js",
    Nomad => "nomad", 0x00ca8e, "Nomad",
    Nvidia => "nvidia", 0x76b900, "NVIDIA",
    Ollama => "ollama", 0x000000, "Ollama",
    // Simple Icons 里没有 OpenAI，取自 gilbarbara/logos（CC0）
    OpenAi => "openai", 0x000000, "OpenAI",
    OpenBsd => "openbsd", 0xf2ca30, "OpenBSD",
    OpenCode => "opencode", 0x000000, "OpenCode",
    Java => "openjdk", 0x000000, "OpenJDK",
    OpenSuse => "opensuse", 0x73ba25, "openSUSE",
    OpenVpn => "openvpn", 0xea7e20, "OpenVPN",
    OpenWrt => "openwrt", 0x00b5e2, "OpenWrt",
    Php => "php", 0x777bb4, "PHP",
    PiHole => "pihole", 0x96060c, "Pi-hole",
    Plex => "plex", 0xebaf00, "Plex",
    Pm2 => "pm2", 0x2b037a, "PM2",
    Podman => "podman", 0x892ca0, "Podman",
    PopOs => "popos", 0x48b9c7, "Pop!_OS",
    Portainer => "portainer", 0x13bef9, "Portainer",
    Postgres => "postgresql", 0x4169e1, "PostgreSQL",
    Prometheus => "prometheus", 0xe6522c, "Prometheus",
    Python => "python", 0x3776ab, "Python",
    QBittorrent => "qbittorrent", 0x2f67ba, "qbittorrent",
    Qemu => "qemu", 0xff6600, "QEMU",
    Qwen => "qwen", 0x6950ef, "Qwen",
    RaspberryPi => "raspberrypi", 0xa22846, "Raspberry Pi",
    Ray => "ray", 0x028cf0, "Ray",
    RedHat => "redhat", 0xee0000, "Red Hat",
    // Redis 用 12.0.0 的经典叠方块标志：16.x 里换成了 2024 年的新「R」，用户看着「不太对」
    Redis => "redis", 0xdc382d, "Redis",
    RockyLinux => "rockylinux", 0x10b981, "Rocky Linux",
    Ruby => "ruby", 0xcc342d, "Ruby",
    Slackware => "slackware", 0x000000, "Slackware",
    Snapcraft => "snapcraft", 0xe95420, "Snapcraft",
    Streamlit => "streamlit", 0xff4b4b, "Streamlit",
    Suse => "suse", 0x0c322c, "SUSE",
    Syncthing => "syncthing", 0x0891d1, "Syncthing",
    Tailscale => "tailscale", 0x242424, "Tailscale",
    Tmux => "tmux", 0x1bb91f, "tmux",
    Traefik => "traefikproxy", 0x24a1c1, "Traefik Proxy",
    Transmission => "transmission", 0xd70008, "Transmission",
    Ubuntu => "ubuntu", 0xe95420, "Ubuntu",
    Vault => "vault", 0xffec6e, "Vault",
    Vaultwarden => "vaultwarden", 0x000000, "Vaultwarden",
    Vim => "vim", 0x019733, "Vim",
    Vllm => "vllm", 0x30a2ff, "vLLM",
    VoidLinux => "voidlinux", 0x478061, "Void Linux",
    WireGuard => "wireguard", 0x88171a, "WireGuard",
    Zorin => "zorin", 0x15a6f0, "Zorin",
}

/// 官方标志是多色或带底形的系统，连接图标画官方彩色版（`assets/icons/brands/color/`，来源和许可见那里的 LICENSE）：
/// 单色路径一涂就走样（Ubuntu 只剩三个小人、Tux 成了一块剪影、CentOS 只剩细线）。
/// 由 img() 画，SVG 的宽高写成 [`COLOR_LOGO_SIZE`]（见下面的测试）。
macro_rules! color_logos {
    ($($variant:ident => $file:literal),* $(,)?) => {
        impl Brand {
            pub fn color_path(self) -> Option<SharedString> {
                match self {
                    $(Brand::$variant => Some(concat!("brands/color/", $file, ".svg").into()),)*
                    _ => None,
                }
            }
        }

        fn color_logo_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("brands/color/", $file, ".svg") => Some(include_bytes!(concat!("../../../assets/icons/brands/color/", $file, ".svg"))),)*
                _ => None,
            }
        }
    };
}

color_logos! {
    AlmaLinux => "almalinux",
    CentOs => "centos",
    Linux => "linux",
    RaspberryPi => "raspberrypi",
    Ubuntu => "ubuntu",
}

/// 彩色标志 SVG 里写的宽高（pt），同侧栏、⌘K 的连接图标。
pub const COLOR_LOGO_SIZE: f32 = 20.;

pub const APP_ICON: &str = "icon.png";
/// 侧栏标志的显示尺寸（pt）。图片按 2 倍屏事先缩好（见下面的测试），换图标后从根目录 icon.png 重新生成：
/// `magick icon.png -colorspace RGB -filter Box -resize 60x60 -colorspace sRGB -strip assets/sidebar-logo.png`
/// （线性光下按面积平均，不用 Lanczos：它在深色方块外缩出一圈浅色光环）
pub const APP_ICON_SIZE: f32 = 30.;
const APP_ICON_BYTES: &[u8] = include_bytes!("../../../assets/sidebar-logo.png");

/// 设置 →「关于」里的应用图标：128 像素的那张按 2 倍屏画成 64pt（侧栏那张只有 60 像素，放大会糊）。
pub const ABOUT_ICON: &str = "about-icon.png";
pub const ABOUT_ICON_SIZE: f32 = 64.;
const ABOUT_ICON_BYTES: &[u8] = include_bytes!("../../../assets/icons/128x128.png");
/// 深色外观用：图标的深色方块和深色卡片几乎一个颜色，边上补一圈 1pt、18% 白的描边（同 macOS 深色下图标的边），
/// 沿着图标自己的透明度算，贴合连续曲率的圆角。换图标后重新生成：
/// `magick 128x128.png -alpha extract a.png; magick a.png -morphology Erode Disk:1.5 e.png; magick a.png e.png -compose minus_src -composite edge.png;`
/// `magick -size 128x128 xc:white \( edge.png -evaluate multiply 0.18 \) -alpha off -compose CopyOpacity -composite ring.png;`
/// `magick 128x128.png ring.png -compose over -composite -strip about-icon-dark.png`
pub const ABOUT_ICON_DARK: &str = "about-icon-dark.png";
const ABOUT_ICON_DARK_BYTES: &[u8] = include_bytes!("../../../assets/icons/about-icon-dark.png");

/// 「关于」页「特别感谢 Voilà Pro」的横幅（同 README 里那张，仓库根目录的 voila-banner.png）。
/// 500×120 像素按 2 倍屏画成 250×60pt，一像素对一像素（见下面的测试）。
pub const VOILA_BANNER: &str = "voila-banner.png";
pub const VOILA_BANNER_SIZE: (f32, f32) = (250., 60.);
const VOILA_BANNER_BYTES: &[u8] = include_bytes!("../../../voila-banner.png");

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(APP_ICON_BYTES)));
        }
        if path == VOILA_BANNER {
            return Ok(Some(Cow::Borrowed(VOILA_BANNER_BYTES)));
        }
        if path == ABOUT_ICON {
            return Ok(Some(Cow::Borrowed(ABOUT_ICON_BYTES)));
        }
        if path == ABOUT_ICON_DARK {
            return Ok(Some(Cow::Borrowed(ABOUT_ICON_DARK_BYTES)));
        }
        Ok(icon_bytes(path).or_else(|| brand_bytes(path)).or_else(|| color_logo_bytes(path)).map(Cow::Borrowed))
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

    /// 「关于」页的图标和横幅同侧栏标志：按 2 倍屏的像素放好，画出来一像素对一像素。
    #[test]
    fn about_images_match_retina_pixel_size() {
        let icon = (ABOUT_ICON_SIZE, ABOUT_ICON_SIZE);
        for (bytes, (width, height)) in [(VOILA_BANNER_BYTES, VOILA_BANNER_SIZE), (ABOUT_ICON_BYTES, icon), (ABOUT_ICON_DARK_BYTES, icon)] {
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            let actual = (u32::from_be_bytes(bytes[16..20].try_into().unwrap()), u32::from_be_bytes(bytes[20..24].try_into().unwrap()));
            assert_eq!(actual, ((width * 2.) as u32, (height * 2.) as u32));
        }
    }

    /// 彩色标志由 img() 画：gpui 按 SVG 自己写的宽高的 2 倍栅格化（`render_single_frame(bytes, 1.)`，同 img 的加载），
    /// 再双线性缩放。宽高写成显示尺寸，2 倍屏上一像素对一像素。
    #[test]
    fn color_logos_rasterize_at_retina_display_size() {
        let renderer = gpui::SvgRenderer::new(std::sync::Arc::new(Assets));
        let expected = gpui::DevicePixels((COLOR_LOGO_SIZE * 2.) as i32);
        let mut count = 0;
        for &brand in Brand::ALL {
            let Some(path) = brand.color_path() else { continue };
            let bytes = Assets.load(&path).unwrap().unwrap_or_else(|| panic!("{path} 不在资源里"));
            let image = renderer.render_single_frame(&bytes, 1.).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(image.size(0), gpui::size(expected, expected), "{path}");
            count += 1;
        }
        assert_eq!(count, 5);
    }
}

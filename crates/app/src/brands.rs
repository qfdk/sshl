//! 画哪个标志：进程名 → 服务端软件（远程服务器上没有应用图标可取，按进程名认出 nginx、redis、postgres、docker、node、python…；
//! 本机不在 .app 里的命令行程序 claude、node 也用，认不出的进程界面画齿轮 / 终端）；连接认出的系统 → 发行版（侧栏的连接图标，
//! 官方标志是多色的画彩色版，见 [`os_logo`]）。

use crate::assets::Brand;
use crate::terminal::frame::Rgb;
use crate::theme::{term::luminance, ui};

/// 进程名（`process_name` 切出来的）就是这些的。
const NAMES: &[(&str, Brand)] = &[
    ("nginx", Brand::Nginx),
    ("apache2", Brand::Apache),
    ("httpd", Brand::Apache),
    ("caddy", Brand::Caddy),
    ("traefik", Brand::Traefik),
    ("haproxy", Brand::HaProxy),
    ("redis", Brand::Redis),
    ("postgres", Brand::Postgres),
    ("postmaster", Brand::Postgres),
    ("psql", Brand::Postgres),
    ("pgbench", Brand::Postgres),
    ("mysqld", Brand::MySql),
    ("mysql", Brand::MySql),
    ("mysqld_safe", Brand::MySql),
    ("mysqldump", Brand::MySql),
    ("mariadbd", Brand::MariaDb),
    ("mariadb", Brand::MariaDb),
    ("mongod", Brand::MongoDb),
    ("mongos", Brand::MongoDb),
    ("mongosh", Brand::MongoDb),
    ("mongodump", Brand::MongoDb),
    ("mongorestore", Brand::MongoDb),
    ("dockerd", Brand::Docker),
    ("docker", Brand::Docker),
    ("docker-proxy", Brand::Docker),
    ("docker-init", Brand::Docker),
    ("kubelet", Brand::Kubernetes),
    ("kubectl", Brand::Kubernetes),
    ("node", Brand::Node),
    ("nodejs", Brand::Node),
    ("npm", Brand::Node),
    ("npx", Brand::Node),
    ("python", Brand::Python),
    ("uvicorn", Brand::Python),
    ("celery", Brand::Python),
    ("pip", Brand::Python),
    ("gunicorn", Brand::Gunicorn),
    ("php", Brand::Php),
    ("php-fpm", Brand::Php),
    ("ruby", Brand::Ruby),
    ("puma", Brand::Ruby),
    ("sidekiq", Brand::Ruby),
    ("unicorn", Brand::Ruby),
    ("java", Brand::Java),
    ("grafana", Brand::Grafana),
    ("grafana-server", Brand::Grafana),
    ("prometheus", Brand::Prometheus),
    ("alertmanager", Brand::Prometheus),
    ("tailscaled", Brand::Tailscale),
    ("tailscale", Brand::Tailscale),
    ("clickhouse", Brand::ClickHouse),
    ("clickhouse-server", Brand::ClickHouse),
    ("influxd", Brand::InfluxDb),
    ("deno", Brand::Deno),
    ("bun", Brand::Bun),
    ("wg", Brand::WireGuard),
    ("wg-quick", Brand::WireGuard),
    ("cloudflared", Brand::Cloudflare),
    ("etcd", Brand::Etcd),
    ("minio", Brand::MinIo),
    ("podman", Brand::Podman),
    ("conmon", Brand::Podman),
    ("pm2", Brand::Pm2),
    ("vault", Brand::Vault),
    ("consul", Brand::Consul),
    ("nomad", Brand::Nomad),
    ("meilisearch", Brand::Meilisearch),
    ("gitea", Brand::Gitea),
    ("openvpn", Brand::OpenVpn),
    ("nats-server", Brand::Nats),
    ("pihole-ftl", Brand::PiHole),
    ("adguardhome", Brand::AdGuard),
    ("envoy", Brand::Envoy),
    ("portainer", Brand::Portainer),
    ("jellyfin", Brand::Jellyfin),
    ("plex", Brand::Plex),
    ("syncthing", Brand::Syncthing),
    ("transmission-daemon", Brand::Transmission),
    ("qbittorrent", Brand::QBittorrent),
    ("qbittorrent-nox", Brand::QBittorrent),
    ("mosquitto", Brand::Mosquitto),
    ("dovecot", Brand::Dovecot),
    // AI 编程工具（node 跑的 gemini、qwen 由 `system_panel::process_name` 按脚本名认）
    ("claude", Brand::Claude),
    ("codex", Brand::OpenAi),
    ("gemini", Brand::Gemini),
    ("copilot", Brand::Copilot),
    ("cursor-agent", Brand::Cursor),
    ("opencode", Brand::OpenCode),
    ("qwen", Brand::Qwen),
    // 跑模型、做数据的
    ("ollama", Brand::Ollama),
    ("vllm", Brand::Vllm),
    ("text-generation-launcher", Brand::HuggingFace),
    ("text-generation-router", Brand::HuggingFace),
    ("streamlit", Brand::Streamlit),
    ("mlflow", Brand::MlFlow),
    ("ray", Brand::Ray),
    ("raylet", Brand::Ray),
    ("n8n", Brand::N8n),
    // 服务器上常见的
    ("snapd", Brand::Snapcraft),
    ("runner.listener", Brand::GitHubActions),
    ("runner.worker", Brand::GitHubActions),
    ("beam.smp", Brand::Erlang),
    ("erl_child_setup", Brand::Erlang),
    ("epmd", Brand::Erlang),
    ("ipfs", Brand::Ipfs),
    ("vaultwarden", Brand::Vaultwarden),
    ("next-server", Brand::NextJs),
    ("next", Brand::NextJs),
    ("vim", Brand::Vim),
    ("vi", Brand::Vim),
    ("nvim", Brand::Neovim),
    ("tmux", Brand::Tmux),
];

/// 进程名以这些开头的（一家软件的一串程序：containerd-shim-runc-v2、kube-apiserver、redis-sentinel、gitlab-runner）。
const PREFIXES: &[(&str, Brand)] = &[
    ("containerd", Brand::Containerd),
    ("kube-", Brand::Kubernetes),
    ("k3s", Brand::K3s),
    ("redis-", Brand::Redis),
    ("gitlab-", Brand::GitLab),
    ("php-fpm", Brand::Php),
    // PostgreSQL 自带的工具：pg_dump、pg_restore、pg_basebackup、pg_ctl
    ("pg_", Brand::Postgres),
    // qemu-ga、qemu-system-x86_64
    ("qemu-", Brand::Qemu),
    // nvidia-persistenced、nvidia-smi
    ("nvidia-", Brand::Nvidia),
    ("jupyter", Brand::Jupyter),
    // 1panel-core、1panel-agent
    ("1panel", Brand::OnePanel),
];

/// 标志的颜色和卡片底色至少差这么多，不然看不见：深色外观下近黑的（Bun、Tailscale、OpenJDK），浅色外观下浅黄的（Vault、ClickHouse）。
const MIN_CONTRAST: f32 = 2.;

/// 进程名认得出是哪个软件。不分大小写；登录 shell 的 `-` 前缀、名字后面的版本号（python3.12、php-fpm8.2）不算；`*_exporter` 算 Prometheus。
pub fn brand_of(name: &str) -> Option<Brand> {
    let name = name.trim_start_matches('-').to_lowercase();
    let bare = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    let exact = |candidate: &str| NAMES.iter().find(|(known, _)| *known == candidate).map(|(_, brand)| *brand);
    exact(&name)
        .or_else(|| exact(bare))
        .or_else(|| PREFIXES.iter().find(|(prefix, _)| name.starts_with(prefix)).map(|(_, brand)| *brand))
        .or_else(|| name.ends_with("_exporter").then_some(Brand::Prometheus))
}

/// 进程列表（卡片上）画这个标志用的颜色（0xRRGGBB），见 [`color_on`]。
pub fn color(brand: Brand, dark: bool) -> u32 {
    color_on(brand, &[ui::CARD], dark)
}

/// 画在这几种底色上（一处地方的平常、选中、悬停）用的颜色：品牌色；和其中任何一种对比不够就用正文色。
/// 同一处的标志不跟着选中 / 悬停换颜色。
pub fn color_on(brand: Brand, surfaces: &[ui::Token], dark: bool) -> u32 {
    let own = luminance(Rgb::hex(brand.hex()));
    let visible = surfaces.iter().all(|surface| contrast(own, luminance(Rgb::hex(surface.resolve(dark)))) >= MIN_CONTRAST);
    if visible { brand.hex() } else { ui::FOREGROUND.resolve(dark) }
}

fn contrast(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// 连接图标上的系统标志怎么画。
#[derive(Clone, Debug, PartialEq)]
pub enum Logo {
    /// 官方彩色版（`Brand::color_path`），用 img() 画。
    Color(gpui::SharedString),
    /// 单色路径（`Brand::path`）涂这个颜色（0xRRGGBB）。
    Tinted(u32),
    /// 单色路径跟着所在位置的文字颜色（同旁边的服务器图标：连着的是正文色，没连的淡一些）。
    Plain,
}

/// 系统标志画在这几种底色上（见 [`color_on`]）。浅色外观：有官方彩色版画彩色版，否则涂品牌色，看不清时用正文色。
/// 深色外观（夜间）：一律纯色（用户要的「纯色模式，给夜间模式用」）——彩色的 Tux 黑身子融进底色、Debian 的暗红发闷，
/// 白色的单色图形就是各家官方给深色底用的那一版（Ubuntu 的白色「朋友圈」）。
pub fn os_logo(brand: Brand, surfaces: &[ui::Token], dark: bool) -> Logo {
    if dark {
        return Logo::Plain;
    }
    match brand.color_path() {
        Some(path) => Logo::Color(path),
        None => Logo::Tinted(color_on(brand, surfaces, dark)),
    }
}

/// 连接记着的系统（`linux/ubuntu`、`darwin`，见 `sshl_core::server_info::parse_os`）画哪个标志：发行版认得的画它的；
/// 认不出的 Linux 发行版（Oracle Linux、Amazon Linux：Simple Icons 里没有）画企鹅，macOS 画苹果。
/// 还没认过、别的系统（NetBSD 的标志是一行字，小了看不清；illumos）是 None，界面画服务器图标。
pub fn of_os(os: &str) -> Option<Brand> {
    let (kernel, distribution) = os.split_once('/').unwrap_or((os, ""));
    let system = match kernel {
        "linux" => Some(Brand::Linux),
        "darwin" => Some(Brand::Apple),
        "freebsd" => Some(Brand::FreeBsd),
        "openbsd" => Some(Brand::OpenBsd),
        _ => None,
    };
    distro(distribution).or(system)
}

/// os-release 的 ID → 发行版的标志。
fn distro(id: &str) -> Option<Brand> {
    Some(match id {
        "ubuntu" => Brand::Ubuntu,
        "debian" => Brand::Debian,
        // 32 位的 Raspberry Pi OS；64 位的写的是 debian
        "raspbian" => Brand::RaspberryPi,
        "centos" => Brand::CentOs,
        "fedora" => Brand::Fedora,
        "rhel" => Brand::RedHat,
        "rocky" => Brand::RockyLinux,
        "almalinux" => Brand::AlmaLinux,
        "alpine" => Brand::AlpineLinux,
        "arch" | "archarm" => Brand::ArchLinux,
        "sles" | "sled" | "sles_sap" | "sle-micro" | "sl-micro" => Brand::Suse,
        "linuxmint" => Brand::LinuxMint,
        "manjaro" | "manjaro-arm" => Brand::Manjaro,
        "kali" => Brand::KaliLinux,
        "gentoo" => Brand::Gentoo,
        "nixos" => Brand::NixOs,
        "pop" => Brand::PopOs,
        "elementary" => Brand::Elementary,
        "zorin" => Brand::Zorin,
        "deepin" => Brand::Deepin,
        "endeavouros" => Brand::EndeavourOs,
        "void" => Brand::VoidLinux,
        "slackware" => Brand::Slackware,
        "openwrt" => Brand::OpenWrt,
        // Alibaba Cloud Linux
        "alinux" => Brand::AlibabaCloud,
        // opensuse-leap、opensuse-tumbleweed、opensuse-microos…
        id if id == "opensuse" || id.starts_with("opensuse-") => Brand::OpenSuse,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 服务器上常见的进程名（ps 的命令行切出来的程序名）。
    #[test]
    fn server_software_is_recognised_by_name() {
        let cases = [
            ("nginx", Some(Brand::Nginx)),
            ("redis-server", Some(Brand::Redis)),
            ("postgres", Some(Brand::Postgres)),
            ("mysqld", Some(Brand::MySql)),
            ("dockerd", Some(Brand::Docker)),
            ("containerd-shim-runc-v2", Some(Brand::Containerd)),
            ("kube-apiserver", Some(Brand::Kubernetes)),
            ("python3.12", Some(Brand::Python)),
            ("python3", Some(Brand::Python)),
            ("php-fpm8.2", Some(Brand::Php)),
            ("node_exporter", Some(Brand::Prometheus)),
            ("pgbench", Some(Brand::Postgres)),
            ("pg_dump", Some(Brand::Postgres)),
            ("mysqldump", Some(Brand::MySql)),
            ("PM2", Some(Brand::Pm2)),
            ("AdGuardHome", Some(Brand::AdGuard)),
            ("claude", Some(Brand::Claude)),
            // 别的 AI 编程工具：原生程序，或者把进程名改成了自己的名字（node 跑的脚本没改名的只认得出是 node）
            ("codex", Some(Brand::OpenAi)),
            ("gemini", Some(Brand::Gemini)),
            ("copilot", Some(Brand::Copilot)),
            ("cursor-agent", Some(Brand::Cursor)),
            ("opencode", Some(Brand::OpenCode)),
            ("qwen", Some(Brand::Qwen)),
            ("ollama", Some(Brand::Ollama)),
            ("vllm", Some(Brand::Vllm)),
            ("text-generation-launcher", Some(Brand::HuggingFace)),
            ("nvidia-persistenced", Some(Brand::Nvidia)),
            ("jupyter-lab", Some(Brand::Jupyter)),
            ("raylet", Some(Brand::Ray)),
            // 用户的服务器上跑着的
            ("snapd", Some(Brand::Snapcraft)),
            ("Runner.Listener", Some(Brand::GitHubActions)),
            ("qemu-ga", Some(Brand::Qemu)),
            ("qemu-system-x86_64", Some(Brand::Qemu)),
            ("beam.smp", Some(Brand::Erlang)),
            ("1panel-core", Some(Brand::OnePanel)),
            ("next-server", Some(Brand::NextJs)),
            ("vaultwarden", Some(Brand::Vaultwarden)),
            ("sshd", None),
            ("bash", None),
            ("-bash", None),
            ("systemd-journald", None),
            ("[kworker/0:1-events]", None),
            ("nodemon", None),
        ];
        for (name, brand) in cases {
            assert_eq!(brand_of(name), brand, "{name}");
        }
    }

    /// 对比不够的换成正文色：深色下近黑的、浅色下浅黄的；够的用品牌色。所有标志在两种外观下、画它的每一处都看得见：
    /// 进程列表（卡片）、侧栏（窗口底，选中项是卡片）、⌘K（卡片，选中行）、设置里的分组树（卡片，悬停行）。
    #[test]
    fn every_logo_stays_visible() {
        assert_eq!(color(Brand::Bun, true), ui::FOREGROUND.resolve(true));
        assert_eq!(color(Brand::Bun, false), Brand::Bun.hex());
        assert_eq!(color(Brand::Vault, false), ui::FOREGROUND.resolve(false));
        assert_eq!(color(Brand::Nginx, false), Brand::Nginx.hex());
        // 深色下 Debian 的红在侧栏上看得清，在 ⌘K 的深蓝选中行上看不清：那一处整个用正文色，不随选中变色
        assert_eq!(color_on(Brand::Debian, &[ui::SHELL, ui::CARD], true), Brand::Debian.hex());
        assert_eq!(color_on(Brand::Debian, &[ui::CARD, ui::ACCENT], true), ui::FOREGROUND.resolve(true));
        // 浅色下企鹅的黄在浅灰底上看不清：画成黑企鹅
        assert_eq!(color_on(Brand::Linux, &[ui::SHELL, ui::CARD], false), ui::FOREGROUND.resolve(false));
        let places: [&[ui::Token]; 4] = [&[ui::CARD], &[ui::SHELL, ui::CARD], &[ui::CARD, ui::ACCENT], &[ui::CARD, ui::MUTED]];
        for dark in [false, true] {
            for surfaces in places {
                for &brand in Brand::ALL {
                    let own = luminance(Rgb::hex(color_on(brand, surfaces, dark)));
                    for surface in surfaces {
                        let ratio = contrast(own, luminance(Rgb::hex(surface.resolve(dark))));
                        assert!(ratio >= MIN_CONTRAST, "{} 在 {surface:?}（深色 {dark}）上只有 {ratio:.2}:1", brand.title());
                    }
                }
            }
        }
    }

    /// 每个标志都打进了二进制、路径对得上（include_bytes 编不过就会漏）。
    #[test]
    fn every_logo_is_bundled() {
        use gpui::AssetSource;
        for &brand in Brand::ALL {
            let bytes = crate::assets::Assets.load(&brand.path()).unwrap().expect("在资源里");
            assert!(bytes.starts_with(b"<svg"), "{}", brand.title());
        }
    }

    /// 连接图标：浅色外观下官方标志是多色 / 带底形的画彩色版，别的涂品牌色（看不清时正文色）；
    /// 深色外观（夜间）下一律纯色，跟着所在位置的文字颜色（用户：「纯色模式，给夜间模式用」）：连着的是白色，没连上的淡一些。
    #[test]
    fn system_logos_are_colourful_by_day_and_plain_at_night() {
        let sidebar: &[ui::Token] = &[ui::SHELL, ui::CARD];
        let color = |brand: Brand| Logo::Color(brand.color_path().unwrap());
        assert_eq!(os_logo(Brand::Ubuntu, sidebar, false), color(Brand::Ubuntu));
        assert_eq!(os_logo(Brand::CentOs, sidebar, false), color(Brand::CentOs));
        assert_eq!(os_logo(Brand::Linux, sidebar, false), color(Brand::Linux));
        assert_eq!(os_logo(Brand::Debian, sidebar, false), Logo::Tinted(Brand::Debian.hex()));
        assert_eq!(os_logo(Brand::Apple, sidebar, false), Logo::Tinted(Brand::Apple.hex()));
        for os in ["linux/ubuntu", "linux/debian", "linux/centos", "linux/ol", "darwin", "freebsd", "linux/opensuse-leap"] {
            let brand = of_os(os).unwrap();
            assert_eq!(os_logo(brand, sidebar, true), Logo::Plain, "{os}");
        }
    }

    /// 连接记着的系统（core 的 `parse_os`）→ 标志。
    #[test]
    fn systems_are_recognised() {
        let cases = [
            ("linux/ubuntu", Some(Brand::Ubuntu)),
            ("linux/debian", Some(Brand::Debian)),
            ("linux/raspbian", Some(Brand::RaspberryPi)),
            ("linux/centos", Some(Brand::CentOs)),
            ("linux/rhel", Some(Brand::RedHat)),
            ("linux/rocky", Some(Brand::RockyLinux)),
            ("linux/almalinux", Some(Brand::AlmaLinux)),
            ("linux/alpine", Some(Brand::AlpineLinux)),
            ("linux/archarm", Some(Brand::ArchLinux)),
            ("linux/opensuse", Some(Brand::OpenSuse)),
            ("linux/opensuse-tumbleweed", Some(Brand::OpenSuse)),
            ("linux/sles", Some(Brand::Suse)),
            ("linux/pop", Some(Brand::PopOs)),
            ("linux/alinux", Some(Brand::AlibabaCloud)),
            // 没有自己标志的发行版画企鹅，不借 ID_LIKE 的（Oracle Linux 的 ID_LIKE 是 fedora，画 Fedora 就错了）
            ("linux/ol", Some(Brand::Linux)),
            ("linux/amzn", Some(Brand::Linux)),
            ("linux/opensusex", Some(Brand::Linux)),
            ("linux", Some(Brand::Linux)),
            ("darwin", Some(Brand::Apple)),
            ("freebsd/freebsd", Some(Brand::FreeBsd)),
            ("openbsd", Some(Brand::OpenBsd)),
            // Debian GNU/Hurd：内核不是 Linux，发行版照样认
            ("gnu/debian", Some(Brand::Debian)),
            ("netbsd", None),
            ("sunos/omnios", None),
            ("", None),
        ];
        for (os, brand) in cases {
            assert_eq!(of_os(os), brand, "{os}");
        }
    }
}

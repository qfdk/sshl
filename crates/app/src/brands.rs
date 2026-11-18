//! 进程名 → 服务端软件的标志。远程服务器上没有应用图标可取（Linux 没有 .app），按进程名认出常见的软件（nginx、redis、postgres、
//! docker、node、python…）画它的标志；本机不在 .app 里的命令行程序（claude、node）也用。认不出的进程界面画齿轮 / 终端。

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
    ("claude", Brand::Claude),
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

/// 画这个标志用的颜色（0xRRGGBB）：品牌色；和卡片底色对比不够就用正文色。
pub fn color(brand: Brand, dark: bool) -> u32 {
    let card = luminance(Rgb::hex(ui::CARD.resolve(dark)));
    let own = luminance(Rgb::hex(brand.hex()));
    let contrast = (own.max(card) + 0.05) / (own.min(card) + 0.05);
    if contrast >= MIN_CONTRAST { brand.hex() } else { ui::FOREGROUND.resolve(dark) }
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

    /// 对比不够的换成正文色：深色下近黑的、浅色下浅黄的；够的用品牌色。所有标志在两种外观下都看得见。
    #[test]
    fn every_logo_stays_visible() {
        assert_eq!(color(Brand::Bun, true), ui::FOREGROUND.resolve(true));
        assert_eq!(color(Brand::Bun, false), Brand::Bun.hex());
        assert_eq!(color(Brand::Vault, false), ui::FOREGROUND.resolve(false));
        assert_eq!(color(Brand::Nginx, false), Brand::Nginx.hex());
        for dark in [false, true] {
            let card = luminance(Rgb::hex(ui::CARD.resolve(dark)));
            for &brand in Brand::ALL {
                let own = luminance(Rgb::hex(color(brand, dark)));
                let contrast = (own.max(card) + 0.05) / (own.min(card) + 0.05);
                assert!(contrast >= MIN_CONTRAST, "{}（深色 {dark}）只有 {contrast:.2}:1", brand.title());
            }
        }
    }

    /// 每个对应到的标志都打进了二进制（include_bytes 编不过就会漏）。
    #[test]
    fn every_mapped_logo_is_bundled() {
        use gpui::AssetSource;
        for (_, brand) in NAMES.iter().chain(PREFIXES) {
            let bytes = crate::assets::Assets.load(&brand.path()).unwrap().expect("在资源里");
            assert!(bytes.starts_with(b"<svg"), "{}", brand.title());
        }
    }
}

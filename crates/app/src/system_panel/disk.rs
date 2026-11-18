//! 「系统」页的「磁盘」：先列各分区用了多少，点一个分区整个扫一遍（`du`，见 `sshl_core::disk_usage`），之后像 ncdu
//! 一样按大小一层层往下点。
//!
//! 要快、不卡：一次只列 4 层（du 照样走完整棵树，总数是准的，输出几万行以内）；点到更深的目录时只补扫那一个目录。
//! 子目录建树时就排好；当前这一层的列表缓存着，换目录、改搜索词、扫描结果变了才重算，每帧只画看得见的几行。
//! du 在服务器上挂在最低的 IO / CPU 优先级下跑。扫描结果按机器和分区留着：切走再回来不用重扫，要新的点「重新扫描」。
//!
//! 本机标签也有：第一次扫描时 macOS 可能会问 SSHL 能不能访问「文稿」「桌面」这些文件夹，不允许的就算不进来。

use std::collections::HashMap;
use std::time::Instant;

use gpui::{Animation, AnimationExt, Transformation, percentage, svg};
use std::sync::atomic::Ordering;

use sshl_core::disk_usage::{self, DiskEntry, DiskTree, Filesystem, ScanProgress};

use super::*;

const USAGE_WIDTH: f32 = 220.;
const SIZE_WIDTH: f32 = 96.;
const SHARE_WIDTH: f32 = 72.;
const STATE_WIDTH: f32 = 120.;

/// 一个分区的扫描。
pub(super) enum Scan {
    Running {
        started: Instant,
        /// 本机并行扫描的进度（远程用 du，一直是 0）。
        progress: Arc<ScanProgress>,
    },
    Done {
        tree: Arc<DiskTree>,
        at: Instant,
        /// 正在补扫的目录（点到了还没列出来的深处）、开始的时间和进度。
        deeper: Option<(String, Instant, Arc<ScanProgress>)>,
        /// 补扫失败的目录和原因。
        failed: Option<(String, String)>,
    },
    Failed(String),
}

/// 当前这一层要画的东西：(机器, 目录, 搜索词, 扫描版本) 变了才重算。
struct Listing {
    key: (Machine, String, String, u64),
    entries: Arc<Vec<DiskEntry>>,
    /// 直接放在这一层的文件（搜索时不显示，算 0）。
    files: u64,
    total: u64,
}

#[derive(Default)]
pub(super) struct DiskState {
    /// 当前机器的分区（每 3 秒随页面刷新）。
    pub(super) filesystems: Option<Vec<Filesystem>>,
    /// (机器, 分区挂载点) → 扫描。
    scans: HashMap<(Machine, String), Scan>,
    /// 每台机器点开了哪个分区、看到了哪一层：(挂载点, 目录)。没有 = 在分区列表。
    at: HashMap<Machine, (String, String)>,
    /// `scans` 每变一次加一：`listing` 据此知道要重算。
    version: u64,
    listing: Option<Listing>,
    scroll: UniformListScrollHandle,
}

// ---------------------------------------------------------------- 纯函数（有测试）

/// 面包屑：从分区根到 `path` 的每一段（显示的名字, 那一段的路径）。根显示成它的挂载点。
pub fn crumbs(root: &str, path: &str) -> Vec<(String, String)> {
    let mut out = vec![(root.to_string(), root.to_string())];
    let rest = path.strip_prefix(root).unwrap_or("");
    let mut current = root.to_string();
    for part in rest.split('/').filter(|part| !part.is_empty()) {
        current = if current == "/" { format!("/{part}") } else { format!("{current}/{part}") };
        out.push((part.to_string(), current.clone()));
    }
    out
}

/// 占这一层的多少。
pub fn share(size: u64, total: u64) -> String {
    if total == 0 || size == 0 {
        return "0%".into();
    }
    let percent = size as f64 * 100. / total as f64;
    if percent < 0.1 { "<0.1%".into() } else if percent >= 10. { format!("{percent:.0}%") } else { format!("{percent:.1}%") }
}

/// 扫描结果有多旧。
pub fn scanned_ago(secs: u64) -> String {
    match secs {
        0..60 => tr!("刚刚扫描").into(),
        60..3600 => tr!("{} 分钟前扫描", secs / 60),
        _ => tr!("{} 小时前扫描", secs / 3600),
    }
}

/// 大数字每三位一个逗号：644498 → 644,498。
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// 搜索：名字里含关键字（不分大小写）。
pub fn filter_entries(entries: Vec<DiskEntry>, query: &str) -> Vec<DiskEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return entries;
    }
    entries.into_iter().filter(|entry| entry.name.to_lowercase().contains(&query)).collect()
}

/// 重新扫描后树只有前几层：`path` 不在树上了就退到树上最深的祖先（不会退出分区根）。
pub fn deepest_known(tree: &DiskTree, mount: &str, path: &str) -> String {
    let mut current = path.to_string();
    while tree.size(&current).is_none() && current.len() > mount.len() {
        current = match current.rsplit_once('/') {
            Some(("", _)) => "/".to_string(),
            Some((parent, _)) => parent.to_string(),
            None => break,
        };
    }
    if current.len() < mount.len() { mount.to_string() } else { current }
}

fn filter_filesystems<'a>(filesystems: &'a [Filesystem], query: &str) -> Vec<&'a Filesystem> {
    let query = query.trim().to_lowercase();
    filesystems.iter().filter(|fs| query.is_empty() || fs.mount.to_lowercase().contains(&query) || fs.device.to_lowercase().contains(&query)).collect()
}

fn spinner(size: f32) -> impl IntoElement {
    svg()
        .path(Icon::LoaderCircle.path())
        .size(px(size))
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .with_animation("disk-spin", Animation::new(Duration::from_secs(1)).repeat(), |svg, delta| {
            svg.with_transformation(Transformation::rotate(percentage(delta)))
        })
}

/// 一条用量条：`ratio` 是 0–1。
fn bar(ratio: f32, color: ui::Token) -> Div {
    let dark = usize::from(theme::is_dark());
    div()
        .flex_1()
        .h(px(6.))
        .rounded_full()
        .bg(theme::alpha(color, RING_TRACK[dark]))
        .child(div().h_full().rounded_full().w(gpui::relative(ratio.clamp(0., 1.))).bg(theme::color(color)))
}

/// 本机：在访达里选中这个目录。测试口开着时只记日志，不去开访达。
pub(super) fn reveal_in_finder(path: &str, cx: &mut App) {
    tracing::info!("reveal: {path}");
    #[cfg(any(debug_assertions, feature = "devhook"))]
    if std::env::var_os("SSHL_DEV_SOCKET").is_some() {
        return;
    }
    cx.reveal_path(std::path::Path::new(path));
}

impl SystemPanel {
    /// 分区列表里的个数（标签上的数字）。
    pub(super) fn disk_count(&self, query: &str) -> Option<usize> {
        self.disk.filesystems.as_deref().map(|filesystems| filter_filesystems(filesystems, query).len())
    }

    /// 点开一个分区：没扫过（或上次失败）就开始扫。
    fn open_filesystem(&mut self, mount: String, cx: &mut Context<Self>) {
        let Some(machine) = self.machine.clone() else { return };
        self.disk.at.insert(machine.clone(), (mount.clone(), mount.clone()));
        self.disk.scroll.scroll_to_item(0, ScrollStrategy::Top);
        if !matches!(self.disk.scans.get(&(machine, mount.clone())), Some(Scan::Running { .. } | Scan::Done { .. })) {
            self.start_scan(mount, cx);
        }
        cx.notify();
    }

    /// 扫整个分区（第一次点开、点「重新扫描」）。
    fn start_scan(&mut self, mount: String, cx: &mut Context<Self>) {
        let Some(machine) = self.machine.clone() else { return };
        let key = (machine.clone(), mount.clone());
        let progress = Arc::new(ScanProgress::default());
        self.disk.scans.insert(key.clone(), Scan::Running { started: Instant::now(), progress: progress.clone() });
        self.disk.version += 1;
        self.tick_while_scanning(key.clone(), cx);
        let scanned = backend::spawn(async move { disk_usage::scan(&backend::get().state, &machine, &mount, progress).await });
        cx.spawn(async move |this, cx| {
            let scan = match scanned.await {
                Some(Ok(tree)) => Scan::Done { tree: Arc::new(tree), at: Instant::now(), deeper: None, failed: None },
                Some(Err(error)) => Scan::Failed(error.to_string()),
                None => Scan::Failed(tr!("后台任务异常退出").into()),
            };
            let _ = this.update(cx, |this, cx| {
                this.disk.scans.insert(key.clone(), scan);
                this.disk.version += 1;
                this.settle_at(&key, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 补扫一个还没列出来的目录，接回这个分区的树上。
    fn scan_deeper(&mut self, key: (Machine, String), path: String, cx: &mut Context<Self>) {
        let Some(Scan::Done { deeper, failed, .. }) = self.disk.scans.get_mut(&key) else { return };
        if deeper.is_some() {
            return;
        }
        let progress = Arc::new(ScanProgress::default());
        *deeper = Some((path.clone(), Instant::now(), progress.clone()));
        *failed = None;
        self.disk.version += 1;
        self.tick_while_scanning(key.clone(), cx);
        let (machine, root) = (key.0.clone(), path.clone());
        let scanned = backend::spawn(async move { disk_usage::scan(&backend::get().state, &machine, &root, progress).await });
        cx.spawn(async move |this, cx| {
            let result = scanned.await;
            let _ = this.update(cx, |this, cx| {
                // 期间点了「重新扫描」：这棵树已经换掉了，结果不要
                let Some(Scan::Done { tree, deeper, failed, .. }) = this.disk.scans.get_mut(&key) else { return };
                if deeper.as_ref().map(|(scanning, ..)| scanning) != Some(&path) {
                    return;
                }
                *deeper = None;
                match result {
                    Some(Ok(sub)) => Arc::make_mut(tree).merge(sub),
                    Some(Err(error)) => *failed = Some((path, error.to_string())),
                    None => *failed = Some((path, tr!("后台任务异常退出").into())),
                }
                this.disk.version += 1;
                cx.notify();
            });
        })
        .detach();
    }

    /// 扫描中每秒画一次（显示用了多久；页面的 3 秒刷新太慢）。
    fn tick_while_scanning(&mut self, key: (Machine, String), cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let scanning = this.update(cx, |this, cx| {
                    cx.notify();
                    matches!(this.disk.scans.get(&key), Some(Scan::Running { .. } | Scan::Done { deeper: Some(_), .. }))
                });
                if !scanning.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// 整个分区扫完：停着的那一层不在新树上（重新扫描时停在深处）就退到树上最深的祖先，它要补扫就补扫。
    fn settle_at(&mut self, key: &(Machine, String), cx: &mut Context<Self>) {
        let Some(Scan::Done { tree, .. }) = self.disk.scans.get(key) else { return };
        let Some((mount, path)) = self.disk.at.get(&key.0).filter(|(mount, _)| *mount == key.1) else { return };
        let settled = deepest_known(tree, mount, path);
        let needs_scan = tree.needs_scan(&settled);
        if let Some(at) = self.disk.at.get_mut(&key.0) {
            at.1 = settled.clone();
        }
        if needs_scan {
            self.scan_deeper(key.clone(), settled, cx);
        }
    }

    fn go_to(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(machine) = self.machine.clone() else { return };
        let Some(at) = self.disk.at.get_mut(&machine) else { return };
        at.1 = path.clone();
        let key = (machine, at.0.clone());
        self.disk.scroll.scroll_to_item(0, ScrollStrategy::Top);
        if let Some(Scan::Done { tree, .. }) = self.disk.scans.get(&key)
            && tree.needs_scan(&path)
        {
            self.scan_deeper(key, path, cx);
        }
        cx.notify();
    }

    fn back_to_filesystems(&mut self, cx: &mut Context<Self>) {
        if let Some(machine) = &self.machine {
            self.disk.at.remove(machine);
            cx.notify();
        }
    }

    /// 当前这一层的列表（见 [`Listing`]）。
    fn listing(&mut self, cx: &App) -> Option<(Arc<Vec<DiskEntry>>, u64, u64)> {
        let machine = self.machine.clone()?;
        let (mount, path) = self.disk.at.get(&machine).cloned()?;
        let query = self.search.read(cx).value().trim().to_string();
        let key = (machine.clone(), path.clone(), query.clone(), self.disk.version);
        if let Some(listing) = self.disk.listing.as_ref().filter(|listing| listing.key == key) {
            return Some((listing.entries.clone(), listing.files, listing.total));
        }
        let Some(Scan::Done { tree, .. }) = self.disk.scans.get(&(machine, mount)) else { return None };
        let entries = Arc::new(filter_entries(tree.entries(&path), &query));
        let files = if query.is_empty() { tree.files_size(&path) } else { 0 };
        let total = tree.size(&path).unwrap_or(0);
        self.disk.listing = Some(Listing { key, entries: entries.clone(), files, total });
        Some((entries, files, total))
    }

    /// 整个「磁盘」页（标签栏下面的部分）。
    pub(super) fn render_disk(&mut self, query: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(machine) = self.machine.clone() else { return empty_message(tr!("连接服务器后显示它的进程、端口和服务")) };
        match self.disk.at.get(&machine).cloned() {
            None => self.render_filesystems(&machine, query, cx),
            Some((mount, path)) => self.render_scan(&machine, mount, path, cx),
        }
    }

    fn render_filesystems(&mut self, machine: &Machine, query: &str, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error) = &self.error {
            return empty_message(list_error_message(error));
        }
        let Some(filesystems) = self.disk.filesystems.as_deref() else { return empty_message(tr!("正在读取…")) };
        let shown = filter_filesystems(filesystems, query);
        if shown.is_empty() {
            return empty_message(if query.trim().is_empty() { tr!("没有内容") } else { tr!("没有匹配的结果") });
        }
        let header = table_header()
            .child(self.header_cell(tr!("分区"), None, None, Align::Start, cx))
            .child(self.header_cell(tr!("用量"), None, Some(USAGE_WIDTH), Align::Start, cx))
            .child(self.header_cell(tr!("已用"), None, Some(SIZE_WIDTH), Align::End, cx))
            .child(self.header_cell(tr!("总量"), None, Some(SIZE_WIDTH), Align::End, cx))
            .child(self.header_cell(tr!("可用"), None, Some(SIZE_WIDTH), Align::End, cx))
            .child(div().w(px(STATE_WIDTH)).flex_shrink_0());
        let rows: Vec<AnyElement> = shown
            .into_iter()
            .enumerate()
            .map(|(index, fs)| {
                let ratio = fs.used as f32 / fs.total.max(1) as f32;
                let color = usage_color(ratio, ui::TEAL);
                let state = match self.disk.scans.get(&(machine.clone(), fs.mount.clone())) {
                    Some(Scan::Running { .. }) => div().flex().items_center().gap(px(6.)).child(spinner(13.)).child(tr!("扫描中…")).into_any_element(),
                    Some(Scan::Done { .. }) => div().text_color(theme::color(ui::PRIMARY)).child(tr!("查看占用")).into_any_element(),
                    _ => div().child(tr!("扫描占用")).into_any_element(),
                };
                let mount = fs.mount.clone();
                table_row(("disk-fs", index))
                    .role(Role::Button)
                    .aria_label(SharedString::from(fs.mount.clone()))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_filesystem(mount.clone(), cx)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            // 挂载点先放全，放不下就截断（不能压到后面几列上）；设备名拿剩下的宽度，先缩没
                            .child(div().flex_shrink_0().max_w_full().truncate().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(fs.mount.clone()))
                            .child(div().min_w_0().truncate().font_family(MONO).text_size(px(11.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(fs.device.clone())),
                    )
                    .child(
                        div()
                            .w(px(USAGE_WIDTH))
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(bar(ratio, color))
                            .child(
                                div()
                                    .w(px(38.))
                                    .flex()
                                    .justify_end()
                                    .font_features(TABULAR.clone())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::color(readable(color)))
                                    .child(format!("{:.0}%", ratio * 100.)),
                            ),
                    )
                    .child(size_cell(fs.used))
                    .child(size_cell(fs.total))
                    .child(size_cell(fs.available))
                    .child(div().w(px(STATE_WIDTH)).flex_shrink_0().px_3().flex().justify_end().text_color(theme::color(ui::MUTED_FOREGROUND)).child(state))
                    .into_any_element()
            })
            .collect();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pb_2()
            .child(header)
            .child(div().id("disk-filesystems").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .into_any_element()
    }

    fn render_scan(&mut self, machine: &Machine, mount: String, path: String, cx: &mut Context<Self>) -> AnyElement {
        // 先把要用的取出来：下面还要可变地借 self 画树
        enum Shown {
            /// 整个分区在扫（秒数, 已扫的文件数, 字节）。
            Scanning(u64, u64, u64),
            /// 这一层在补扫。
            Deeper(u64, u64, u64),
            Failed(String),
            Done(Instant, usize),
        }
        let counted = |started: &Instant, progress: &ScanProgress| {
            (started.elapsed().as_secs(), progress.files.load(Ordering::Relaxed), progress.bytes.load(Ordering::Relaxed))
        };
        let shown = match self.disk.scans.get(&(machine.clone(), mount.clone())) {
            None => Shown::Scanning(0, 0, 0),
            Some(Scan::Running { started, progress }) => {
                let (secs, files, bytes) = counted(started, progress);
                Shown::Scanning(secs, files, bytes)
            }
            Some(Scan::Failed(error)) => Shown::Failed(error.clone()),
            Some(Scan::Done { deeper: Some((scanning, started, progress)), .. }) if *scanning == path => {
                let (secs, files, bytes) = counted(started, progress);
                Shown::Deeper(secs, files, bytes)
            }
            Some(Scan::Done { failed: Some((at, error)), .. }) if *at == path => Shown::Failed(error.clone()),
            Some(Scan::Done { tree, at, .. }) => Shown::Done(*at, tree.unreadable),
        };
        let local = *machine == Machine::Local;
        let rescan_mount = mount.clone();
        let full_scan = matches!(shown, Shown::Scanning(..));
        // 面包屑：所有分区 › 分区 › 目录 › 目录 … 右边是重新扫描
        let crumbs_bar = div()
            .flex_shrink_0()
            .px_3()
            .pb_2()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            .child(
                div()
                    .id("disk-back")
                    .role(Role::Button)
                    .aria_label(tr!("所有分区"))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_color(theme::color(ui::PRIMARY))
                    .hover(|s| s.bg(theme::alpha(ui::PRIMARY, 0.08)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.back_to_filesystems(cx)))
                    .child(kit::icon_sized(Icon::Server, 13.))
                    .child(tr!("所有分区")),
            )
            .children(crumbs(&mount, &path).into_iter().enumerate().map(|(index, (name, target))| {
                let last = target == path;
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .child(kit::icon_sized(Icon::ChevronRight, 12.).text_color(theme::color(ui::MUTED_FOREGROUND)))
                    .child(
                        div()
                            .id(("disk-crumb", index))
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .px(px(4.))
                            .rounded(px(5.))
                            .map(|d| {
                                if last {
                                    d.font_weight(FontWeight::SEMIBOLD)
                                } else {
                                    d.cursor_pointer()
                                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                                        .hover(|s| s.text_color(theme::color(ui::FOREGROUND)).bg(theme::alpha(ui::FOREGROUND, 0.06)))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go_to(target.clone(), cx)))
                                }
                            })
                            .child(name),
                    )
            }))
            .child(div().flex_1())
            .when(!full_scan, |d| {
                d.child(
                    kit::button("disk-rescan", kit::Variant::Outline)
                        .h(px(28.))
                        .px_3()
                        .text_size(px(12.))
                        .aria_label(tr!("重新扫描"))
                        .child(div().flex().items_center().gap(px(6.)).child(kit::icon_sized(Icon::RefreshCw, 12.)).child(tr!("重新扫描")))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.start_scan(rescan_mount.clone(), cx))),
                )
            });

        // 本机并行扫描有进度：已扫多少文件、多大（远程用 du，拿不到，只显示用了几秒）
        let scanning = |target: &str, (secs, files, bytes): (u64, u64, u64), hints: Vec<&'static str>| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .px_4()
                .child(spinner(24.))
                .child(div().text_sm().child(tr!("正在扫描 {}…已用 {} 秒", target, secs)))
                .when(files > 0, |d| {
                    d.child(div().text_sm().font_features(TABULAR.clone()).text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("已扫 {} 个文件 · {}", group_digits(files), format_bytes(bytes))))
                })
                .children(hints.into_iter().map(|hint| div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(hint)))
                .into_any_element()
        };
        let body = match shown {
            Shown::Scanning(secs, files, bytes) => {
                let mut hints = vec![tr!("分区越大越久。扫完后往下点都是现成的，不用再等。")];
                if local {
                    hints.push(tr!("第一次扫描时 macOS 可能会问 SSHL 能不能访问「文稿」「桌面」等文件夹；不允许的话这些文件夹算不进来。"));
                }
                scanning(&mount, (secs, files, bytes), hints)
            }
            Shown::Deeper(secs, files, bytes) => scanning(&path, (secs, files, bytes), Vec::new()),
            Shown::Failed(error) => empty_message(tr!("扫描失败：{}", error.trim())),
            Shown::Done(at, unreadable) => self.render_tree(at, unreadable, local, cx),
        };
        div().flex_1().min_h_0().flex().flex_col().child(crumbs_bar).child(body).into_any_element()
    }

    fn render_tree(&mut self, at: Instant, unreadable: usize, local: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some((entries, files, _)) = self.listing(cx) else { return empty_message(tr!("正在读取…")) };
        let query = self.search.read(cx).value().trim().to_string();
        let rows = entries.len() + usize::from(files > 0);
        let mut note = scanned_ago(at.elapsed().as_secs());
        if unreadable > 0 {
            let reason = if local {
                tr!("有 {} 个目录没有权限读（系统保护的，或没允许 SSHL 访问的），实际占用比这里大。", unreadable)
            } else {
                tr!("有 {} 个目录没有权限读，实际占用比这里大。在终端里 sudo -i 之后重新扫描就能看全。", unreadable)
            };
            note = format!("{note} · {reason}");
        }
        let header = table_header()
            .child(self.header_cell(tr!("名称"), None, None, Align::Start, cx))
            .child(self.header_cell(tr!("占用"), None, Some(USAGE_WIDTH), Align::Start, cx))
            .child(self.header_cell(tr!("大小"), None, Some(SIZE_WIDTH), Align::End, cx))
            .child(self.header_cell(tr!("占比"), None, Some(SHARE_WIDTH), Align::End, cx));
        let body = if rows == 0 {
            empty_message(if query.is_empty() { tr!("这个目录是空的") } else { tr!("没有匹配的结果") })
        } else {
            uniform_list("disk-rows", rows, cx.processor(move |this, range: Range<usize>, _, cx| this.render_tree_rows(range, cx)))
                .track_scroll(&self.disk.scroll)
                .flex_1()
                .w_full()
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pb_2()
            .child(div().flex_shrink_0().px_4().pb_2().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(note))
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_tree_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(machine) = self.machine.clone() else { return Vec::new() };
        let Some((entries, files, total)) = self.listing(cx) else { return Vec::new() };
        // 条的长度按这一层最大的那项算，看得出谁大；占比按这一层的总量
        let largest = entries.first().map_or(0, |entry| entry.size).max(files).max(1);
        let remote = matches!(machine, Machine::Remote(_));
        range
            .map(|ix| match entries.get(ix) {
                Some(entry) => {
                    let (target, menu_path) = (entry.path.clone(), entry.path.clone());
                    let has_children = entry.has_children;
                    table_row(("disk-entry", ix))
                        .when(has_children, |row| {
                            row.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go_to(target.clone(), cx)))
                        })
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.open_menu(event.position, MenuTarget::Directory { path: menu_path.clone(), remote }, cx);
                            }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .px_3()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(kit::icon_sized(Icon::Folder, 14.).text_color(theme::color(ui::PRIMARY)))
                                .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(entry.name.clone()))
                                .when(has_children, |d| d.child(kit::icon_sized(Icon::ChevronRight, 12.).text_color(theme::color(ui::MUTED_FOREGROUND)))),
                        )
                        .child(usage_cell(entry.size as f32 / largest as f32, ui::PRIMARY))
                        .child(size_cell(entry.size))
                        .child(share_cell(entry.size, total))
                        .into_any_element()
                }
                // 最后一行：直接放在这一层的文件
                None => table_row(("disk-files", ix))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                            .child(kit::icon_sized(Icon::FileText, 14.))
                            .child(tr!("这一层的文件")),
                    )
                    .child(usage_cell(files as f32 / largest as f32, ui::MUTED_FOREGROUND))
                    .child(size_cell(files))
                    .child(share_cell(files, total))
                    .into_any_element(),
            })
            .collect()
    }
}

fn empty_message(message: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .text_sm()
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .child(message.into())
        .into_any_element()
}

fn size_cell(bytes: u64) -> Div {
    div().w(px(SIZE_WIDTH)).flex_shrink_0().px_3().flex().justify_end().font_features(TABULAR.clone()).child(format_bytes(bytes))
}

fn share_cell(size: u64, total: u64) -> Div {
    div()
        .w(px(SHARE_WIDTH))
        .flex_shrink_0()
        .px_3()
        .flex()
        .justify_end()
        .font_features(TABULAR.clone())
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .child(share(size, total))
}

fn usage_cell(ratio: f32, color: ui::Token) -> Div {
    div().w(px(USAGE_WIDTH)).flex_shrink_0().px_3().flex().items_center().child(bar(ratio, color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crumbs_walk_from_the_mount_point_down() {
        let names = |root: &str, path: &str| crumbs(root, path).into_iter().map(|(name, path)| format!("{name}={path}")).collect::<Vec<_>>();
        assert_eq!(names("/", "/"), ["/=/"]);
        assert_eq!(names("/", "/var/lib/docker"), ["/=/", "var=/var", "lib=/var/lib", "docker=/var/lib/docker"]);
        assert_eq!(names("/data", "/data/backups/2026"), ["/data=/data", "backups=/data/backups", "2026=/data/backups/2026"]);
    }

    #[test]
    fn shares_read_well() {
        assert_eq!(share(50, 100), "50%");
        assert_eq!(share(5, 100), "5.0%");
        assert_eq!(share(1, 10_000), "<0.1%");
        assert_eq!(share(0, 100), "0%");
        assert_eq!(share(10, 0), "0%", "空目录不除以 0");
    }

    #[test]
    fn scan_age_is_rounded_for_people() {
        assert_eq!(scanned_ago(5), "刚刚扫描");
        assert_eq!(scanned_ago(150), "2 分钟前扫描");
        assert_eq!(scanned_ago(7300), "2 小时前扫描");
    }

    #[test]
    fn big_counts_get_thousands_separators() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(644498), "644,498");
        assert_eq!(group_digits(4544185), "4,544,185");
    }

    #[test]
    fn entries_are_searched_by_name() {
        let entry = |name: &str| DiskEntry { name: name.into(), path: format!("/{name}"), size: 1, has_children: false };
        let found = filter_entries(vec![entry("Docker"), entry("log"), entry("lib")], "dock");
        assert_eq!(found.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Docker"]);
        assert_eq!(filter_entries(vec![entry("a"), entry("b")], "  ").len(), 2);
    }

    /// 在深处点了「重新扫描」：新树只有前 4 层，退到树上最深的祖先，不退出分区。
    #[test]
    fn a_rescan_falls_back_to_the_deepest_known_folder() {
        let tree = disk_usage::parse_du("10\t/data/a/b/c/d\n20\t/data/a/b/c\n30\t/data/a/b\n40\t/data/a\n50\t/data\n", "/data", 0, 4);
        assert_eq!(deepest_known(&tree, "/data", "/data/a/b/c/d/e/f"), "/data/a/b/c/d");
        assert_eq!(deepest_known(&tree, "/data", "/data/a/b"), "/data/a/b", "在树上的不动");
        assert_eq!(deepest_known(&tree, "/data", "/data/gone/x"), "/data");
        let root = disk_usage::parse_du("5\t/\n", "/", 0, 4);
        assert_eq!(deepest_known(&root, "/", "/var/lib"), "/");
    }
}

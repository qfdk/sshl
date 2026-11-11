//! 连接分组与排序（旧 lib/connection-groups.ts、lib/session-tabs.ts 的原生版）。

use sshl_core::config_store::StoredConnection;

pub fn normalize(group: &str) -> &str {
    group.trim()
}

pub struct Group<'a> {
    pub name: String,
    pub connections: Vec<&'a StoredConnection>,
}

pub fn ungrouped<'a>(connections: &[&'a StoredConnection]) -> Vec<&'a StoredConnection> {
    connections.iter().copied().filter(|c| normalize(&c.group).is_empty()).collect()
}

/// 有名字的分组：先按显式顺序，没列进顺序的按首次出现排在后面。空分组也保留（刚建的分组要能拖进去）。
pub fn group_connections<'a>(connections: &[&'a StoredConnection], order: &[String]) -> Vec<Group<'a>> {
    let mut names: Vec<String> = Vec::new();
    for name in order.iter().map(|n| normalize(n)) {
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    for conn in connections {
        let name = normalize(&conn.group);
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
        .into_iter()
        .map(|name| Group {
            connections: connections.iter().copied().filter(|c| normalize(&c.group) == name).collect(),
            name,
        })
        .collect()
}

/// 把连接移到 `target` 分组（空串 = 移出分组），放在 `before` 之前（None = 末尾）。返回新的整体顺序。
pub fn move_connection(
    connections: &[StoredConnection],
    order: &[String],
    id: &str,
    target: &str,
    before: Option<&str>,
) -> Vec<StoredConnection> {
    let Some(source) = connections.iter().find(|c| c.id == id) else {
        return connections.to_vec();
    };
    let target = normalize(target).to_string();
    let mut moved = source.clone();
    moved.group = target.clone();
    let remaining: Vec<&StoredConnection> = connections.iter().filter(|c| c.id != id).collect();

    let mut default: Vec<StoredConnection> = remaining.iter().filter(|c| normalize(&c.group).is_empty()).map(|c| (*c).clone()).collect();
    let mut groups = group_connections(&remaining, order)
        .into_iter()
        .map(|g| (g.name, g.connections.into_iter().cloned().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    // 目标分组当前为空且不在顺序里时也要有桶，否则被拖动的连接会整条消失
    if !target.is_empty() && !groups.iter().any(|(name, _)| *name == target) {
        groups.push((target.clone(), Vec::new()));
    }
    let bucket = if target.is_empty() {
        &mut default
    } else {
        &mut groups.iter_mut().find(|(name, _)| *name == target).expect("bucket exists").1
    };
    let at = before.and_then(|before| bucket.iter().position(|c| c.id == before));
    match at {
        Some(at) => bucket.insert(at, moved),
        None => bucket.push(moved),
    }
    let mut ordered = default;
    for (_, items) in groups {
        ordered.extend(items);
    }
    ordered
}

pub fn reorder_groups(groups: &[String], dragged: &str, target: &str, after: bool) -> Vec<String> {
    let (Some(from), Some(to)) = (groups.iter().position(|g| g == dragged), groups.iter().position(|g| g == target)) else {
        return groups.to_vec();
    };
    if from == to {
        return groups.to_vec();
    }
    let mut out = groups.to_vec();
    let item = out.remove(from);
    let to = if to > from { to - 1 } else { to };
    out.insert(to + usize::from(after), item);
    out
}

/// 搜索：名称、主机、用户名、分组任一包含（不分大小写）。
pub fn matches(conn: &StoredConnection, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    [&conn.name, &conn.host, &conn.username, &conn.group].iter().any(|v| v.to_lowercase().contains(&needle))
}

/// 名称旁边的主机名：连接就是以主机名命名时不再重复显示一遍。
pub fn distinct_host<'a>(name: &str, host: &'a str) -> &'a str {
    if host.is_empty() || name.trim().eq_ignore_ascii_case(host.trim()) { "" } else { host }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Before,
    After,
}

/// 关掉当前标签后该切到哪个：`remaining` 是去掉它之后的标签，`closed_at` 是它原来的位置。
/// 同浏览器：右边的补上来，关的是最后一个就退到左边。
pub fn tab_after_close<T: Clone>(remaining: &[T], closed_at: usize) -> Option<T> {
    remaining.get(closed_at).or_else(|| remaining.last()).cloned()
}

/// 把拖动的标签放到目标标签的前或后；拖到自己身上或 id 不存在时原样返回。
pub fn move_tab<T: PartialEq + Clone>(ids: &[T], dragged: &T, target: &T, side: Side) -> Vec<T> {
    if dragged == target || !ids.contains(dragged) || !ids.contains(target) {
        return ids.to_vec();
    }
    let mut rest: Vec<T> = ids.iter().filter(|id| *id != dragged).cloned().collect();
    let at = rest.iter().position(|id| id == target).unwrap() + usize::from(side == Side::After);
    rest.insert(at, dragged.clone());
    rest
}

/// ⌘⇧] / ⌘⇧[（⌃Tab / ⌃⇧Tab）：从 `current` 往右 / 左数 `step` 个标签，两头绕回。
pub fn cycle_tab<T: PartialEq + Clone>(ids: &[T], current: Option<&T>, step: isize) -> Option<T> {
    if ids.is_empty() {
        return None;
    }
    let at = current.and_then(|c| ids.iter().position(|id| id == c)).map_or(0, |i| i as isize + step);
    ids.get(at.rem_euclid(ids.len() as isize) as usize).cloned()
}

/// ⌘1 … ⌘9：第 `n` 个标签（从 1 数，标签栏从左到右）；没有那么多标签（或 n 是 0）是 None。
pub fn nth_tab<T: Clone>(ids: &[T], n: usize) -> Option<T> {
    n.checked_sub(1).and_then(|at| ids.get(at)).cloned()
}

/// 标签自定义名字最多几个字（按字符数，中英文都算一个）：再长标签栏也放不下，多出来的截掉。
pub const TAB_TITLE_MAX: usize = 40;

/// 标签上显示的名字：改过名的用自定义名，否则用默认名字（连接名 / 「本机」）。
pub fn tab_title<'a>(custom: Option<&'a str>, default: &'a str) -> &'a str {
    custom.unwrap_or(default)
}

/// 重命名确认时的输入 → 自定义名：去掉首尾空白，超过 [`TAB_TITLE_MAX`] 个字的截掉；
/// 空的、或和默认名字一样的 = 不要自定义名（None），标签回到默认名字。
pub fn tab_custom_title(input: &str, default: &str) -> Option<String> {
    let capped: String = input.trim().chars().take(TAB_TITLE_MAX).collect();
    let title = capped.trim_end();
    (!title.is_empty() && title != default).then(|| title.to_string())
}

/// 新本机标签的名字：「本机」，已有的话「本机 2」「本机 3」…，取最小的没用过的。
pub fn local_tab_name(existing: &[String]) -> String {
    (1..)
        .map(|n| if n == 1 { "本机".to_string() } else { format!("本机 {n}") })
        .find(|name| !existing.contains(name))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_tabs_wraps_around_both_ends() {
        let ids = ["a", "b", "c"];
        assert_eq!(cycle_tab(&ids, Some(&"c"), 1), Some("a"));
        assert_eq!(cycle_tab(&ids, Some(&"a"), -1), Some("c"));
        assert_eq!(cycle_tab(&ids, Some(&"a"), 1), Some("b"));
        assert_eq!(cycle_tab(&ids, None, 1), Some("a"), "没有当前标签时从第一个开始");
        assert_eq!(cycle_tab::<&str>(&[], None, 1), None);
    }

    #[test]
    fn cmd_number_picks_the_nth_tab_from_the_left() {
        let ids = ["a", "b", "c"];
        assert_eq!(nth_tab(&ids, 1), Some("a"));
        assert_eq!(nth_tab(&ids, 3), Some("c"));
        assert_eq!(nth_tab(&ids, 4), None, "没有第 4 个标签：什么都不做");
        assert_eq!(nth_tab(&ids, 0), None);
        assert_eq!(nth_tab::<&str>(&[], 1), None);
    }

    #[test]
    fn a_custom_tab_title_wins_over_the_default() {
        assert_eq!(tab_title(Some("生产库"), "web-01"), "生产库");
        assert_eq!(tab_title(None, "web-01"), "web-01");
        assert_eq!(tab_title(None, "本机 2"), "本机 2");
    }

    #[test]
    fn renaming_trims_the_input_and_blank_restores_the_default() {
        assert_eq!(tab_custom_title("  生产库 ", "web-01").as_deref(), Some("生产库"));
        assert_eq!(tab_custom_title("a  b", "web-01").as_deref(), Some("a  b"), "中间的空格留着");
        assert_eq!(tab_custom_title("", "web-01"), None);
        assert_eq!(tab_custom_title(" \t  ", "web-01"), None, "全是空白 = 恢复默认名字");
        assert_eq!(tab_custom_title(" web-01 ", "web-01"), None, "改回默认名字也就是没有自定义名");
    }

    #[test]
    fn overlong_tab_titles_are_cut_at_the_limit() {
        let long = "长".repeat(TAB_TITLE_MAX + 5);
        assert_eq!(tab_custom_title(&long, "web").unwrap().chars().count(), TAB_TITLE_MAX);
        let exact = "x".repeat(TAB_TITLE_MAX);
        assert_eq!(tab_custom_title(&exact, "web").as_deref(), Some(exact.as_str()));
        // 截断处正好是空格：截完不留尾巴上的空白
        let spaced = format!("{} tail", "y".repeat(TAB_TITLE_MAX - 1));
        assert_eq!(tab_custom_title(&spaced, "web").unwrap(), "y".repeat(TAB_TITLE_MAX - 1));
    }

    #[test]
    fn local_tabs_take_the_smallest_free_number() {
        assert_eq!(local_tab_name(&[]), "本机");
        assert_eq!(local_tab_name(&["本机".into()]), "本机 2");
        assert_eq!(local_tab_name(&["本机".into(), "本机 3".into()]), "本机 2");
        assert_eq!(local_tab_name(&["本机 2".into()]), "本机");
    }

    fn conn(id: &str, group: &str) -> StoredConnection {
        StoredConnection {
            id: id.into(),
            name: id.into(),
            host: format!("{id}.example"),
            port: 22,
            username: "root".into(),
            private_key: None,
            has_password: false,
            has_passphrase: false,
            group: group.into(),
            auth: Default::default(),
        }
    }

    fn ids(list: &[StoredConnection]) -> Vec<&str> {
        list.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn orders_named_groups_and_leaves_ungrouped_at_top_level() {
        let all = [conn("a", "prod"), conn("b", ""), conn("c", "dev")];
        let refs: Vec<_> = all.iter().collect();
        let groups = group_connections(&refs, &["dev".into(), "prod".into()]);
        assert_eq!(groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["dev", "prod"]);
        assert_eq!(ungrouped(&refs).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn moves_between_groups_preserving_target_position() {
        let all = [conn("a", ""), conn("b", "prod"), conn("c", "prod")];
        let result = move_connection(&all, &["prod".into()], "a", "prod", Some("c"));
        assert_eq!(ids(&result), ["b", "a", "c"]);
        assert_eq!(result[1].group, "prod");
    }

    #[test]
    fn reorders_groups_without_dropping_names() {
        let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(reorder_groups(&g(&["a", "b", "c"]), "a", "c", false), g(&["b", "a", "c"]));
        assert_eq!(reorder_groups(&g(&["a", "b", "c"]), "c", "a", true), g(&["a", "c", "b"]));
    }

    #[test]
    fn search_matches_fields_and_group() {
        let item = conn("server", "production");
        assert!(matches(&item, "PROD"));
        assert!(!matches(&item, "missing"));
    }

    #[test]
    fn moves_into_an_empty_group_without_dropping_it() {
        let all = [conn("a", ""), conn("b", "")];
        let result = move_connection(&all, &["fresh".into()], "a", "fresh", None);
        let mut got = ids(&result);
        got.sort();
        assert_eq!(got, ["a", "b"]);
        assert_eq!(result.iter().find(|c| c.id == "a").unwrap().group, "fresh");
    }

    #[test]
    fn moves_into_a_group_missing_from_the_order() {
        let result = move_connection(&[conn("a", "")], &[], "a", "fresh", None);
        assert_eq!(ids(&result), ["a"]);
        assert_eq!(result[0].group, "fresh");
    }

    #[test]
    fn host_is_omitted_when_the_connection_is_named_after_it() {
        assert_eq!(distinct_host("fro1.qfdk.me", "fro1.qfdk.me"), "");
        assert_eq!(distinct_host(" FRO2.qfdk.me ", "fro2.qfdk.me"), "");
        assert_eq!(distinct_host("web-01", "10.0.1.11"), "10.0.1.11");
    }

    #[test]
    fn moves_a_dragged_tab_before_or_after_the_target() {
        let ids = ["a", "b", "c", "d"];
        assert_eq!(move_tab(&ids, &"d", &"b", Side::Before), ["a", "d", "b", "c"]);
        assert_eq!(move_tab(&ids, &"a", &"c", Side::After), ["b", "c", "a", "d"]);
        assert_eq!(move_tab(&ids, &"a", &"d", Side::After), ["b", "c", "d", "a"]);
        assert_eq!(move_tab(&ids, &"b", &"b", Side::Before), ids);
        assert_eq!(move_tab(&ids, &"x", &"b", Side::Before), ids);
    }

    #[test]
    fn closing_the_active_tab_moves_to_its_right_then_left_neighbour() {
        // 关掉 [a b c] 里的 b：右边的 c 补上来
        assert_eq!(tab_after_close(&["a", "c"], 1), Some("c"));
        // 关掉最后一个 c：退到左边的 b
        assert_eq!(tab_after_close(&["a", "b"], 2), Some("b"));
        // 关掉第一个 a
        assert_eq!(tab_after_close(&["b", "c"], 0), Some("b"));
        // 最后一个标签也关了：没有可切的
        assert_eq!(tab_after_close::<&str>(&[], 0), None);
    }
}

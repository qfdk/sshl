//! 连接分组与排序（旧 lib/connection-groups.ts、lib/session-tabs.ts 的原生版）。

use sshl_core::config_store::StoredConnection;
use unicode_segmentation::UnicodeSegmentation;

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

/// 侧栏收起后的方块：名称的第一个字形簇（emoji 开头就是那个 emoji），字母大写。
pub fn initials(name: &str, host: &str) -> String {
    let source = if name.trim().is_empty() { host.trim() } else { name.trim() };
    source.graphemes(true).next().map_or_else(|| "?".to_string(), |g| g.to_uppercase())
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn initials_take_the_first_grapheme() {
        assert_eq!(initials("prod-web-01", ""), "P");
        assert_eq!(initials("生产数据库", ""), "生");
        assert_eq!(initials("🚀 deploy", ""), "🚀");
        assert_eq!(initials("👨‍💻 dev", ""), "👨‍💻");
        assert_eq!(initials("", "db.example.com"), "D");
        assert_eq!(initials("", ""), "?");
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
}

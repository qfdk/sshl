//! 文件管理跟随终端用户（旧 file-user.ts）：终端里 su / sudo -i 成了 root，文件管理就以 root 身份浏览、上传、下载、删除。
//! 后端 file_terminal_user 从这条连接的进程链看出终端此刻是谁、怎么切过去的；file_switch_user 以他的身份起 sftp-server。
//! 密码：su 要目标用户的，sudo 要登录用户的；先用那位用户自己的已保存密码，没有再弹框，不拿别的已保存密码去试。

use std::collections::{HashMap, HashSet};

use super::sudo::{Outcome, Secret, SudoFailure, retry_with_password, sudo_failure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Su,
    Sudo,
}

impl Via {
    pub fn parse(value: &str) -> Option<Via> {
        match value {
            "su" => Some(Via::Su),
            "sudo" => Some(Via::Sudo),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Via::Su => "su",
            Via::Sudo => "sudo",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalIdentity {
    pub login_user: String,
    /// 终端此刻的用户。
    pub user: String,
    pub via: Option<Via>,
    /// 这次需要的那份密码有没有保存（标识）。
    pub credential: Option<String>,
    /// 文件管理当前以谁的身份运行。
    pub file_user: String,
}

impl From<sshl_core::sftp::TerminalIdentity> for TerminalIdentity {
    fn from(who: sshl_core::sftp::TerminalIdentity) -> Self {
        Self { login_user: who.login_user, user: who.user, via: who.via.and_then(Via::parse), credential: who.credential, file_user: who.file_user }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchTarget {
    pub user: String,
    pub via: Via,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordContext {
    pub user: String,
    pub via: Via,
    /// 要谁的密码：su → 目标用户，sudo → 登录用户。
    pub need_user: String,
    pub login_user: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    Same,
    Switched,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncResult {
    pub status: SyncStatus,
    pub file_user: Option<String>,
    pub terminal_user: Option<String>,
    pub error: Option<String>,
    /// 这次自动试了谁的已保存密码、服务器认不认（没试或没得出结论时没有）。
    pub saved: Option<(String, bool)>,
}

impl SyncResult {
    fn new(status: SyncStatus, file_user: Option<String>, terminal_user: Option<String>) -> Self {
        Self { status, file_user, terminal_user, error: None, saved: None }
    }
}

/// `interactive`：点徽标（总要问）还是自动同步（切到文件管理 / 刷新时）。
/// `declined_user`：自动同步时用户取消过输密码的那位终端用户：没有可自动用的密码就不再弹框打扰。
/// `rejected_saved`：这个会话里已保存密码自动试过、被服务器拒绝的用户（按要谁的密码记）。
pub async fn sync_file_user(
    identify: impl AsyncFnOnce() -> Option<TerminalIdentity>,
    mut switch_user: impl AsyncFnMut(Option<SwitchTarget>, Option<Secret>) -> Outcome,
    mut ask_password: impl AsyncFnMut(bool, Vec<String>, PasswordContext) -> Option<Secret>,
    interactive: bool,
    declined_user: Option<&str>,
    rejected_saved: &HashSet<String>,
) -> SyncResult {
    let Some(who) = identify().await else {
        return SyncResult::new(SyncStatus::Same, None, None);
    };
    let unchanged = |status| SyncResult::new(status, Some(who.file_user.clone()), Some(who.user.clone()));
    if who.user == who.file_user {
        return unchanged(SyncStatus::Same);
    }
    let Some(via) = who.via else {
        // 终端 exit 回到了登录用户
        let back = switch_user(None, None).await;
        return match back {
            Outcome::Ok => SyncResult::new(SyncStatus::Switched, Some(who.login_user.clone()), Some(who.user.clone())),
            other => SyncResult { error: other.error().map(String::from), ..unchanged(SyncStatus::Failed) },
        };
    };
    let need_user = if via == Via::Su { who.user.clone() } else { who.login_user.clone() };
    // 已知不对的已保存密码，自动同步不再去试：每次失败的 su / sudo 都进服务器认证日志，可能触发锁号
    let skipped = if !interactive && rejected_saved.contains(&need_user) { who.credential.clone() } else { None };
    let own = if skipped.is_some() { None } else { who.credential.clone() };
    if !interactive && declined_user == Some(who.user.as_str()) && own.is_none() {
        return unchanged(SyncStatus::Cancelled);
    }
    let target = SwitchTarget { user: who.user.clone(), via };
    let context = PasswordContext { user: who.user.clone(), via, need_user: need_user.clone(), login_user: who.login_user.clone() };
    let mut saved: Option<(String, bool)> = None;
    let mut first = true;
    let auto = own.is_some();
    let result = retry_with_password(
        async |secret| {
            let attempt = switch_user(Some(target.clone()), secret).await;
            // 第一次是自动用他自己的已保存密码（有的话）：记下服务器认不认它
            if first && auto {
                if attempt == Outcome::Ok {
                    saved = Some((need_user.clone(), true));
                } else if sudo_failure(attempt.error()) == SudoFailure::PasswordIncorrect {
                    saved = Some((need_user.clone(), false));
                }
            }
            first = false;
            attempt
        },
        own,
        // 跳过的那个错密码也不列进弹框的可选项（同一次里试错的本来就不列）
        async |retry, mut exclude| {
            if let Some(skipped) = &skipped
                && !exclude.contains(skipped)
            {
                exclude.insert(0, skipped.clone());
            }
            ask_password(retry, exclude, context.clone()).await
        },
    )
    .await;
    let mut out = match &result {
        Outcome::Ok => SyncResult::new(SyncStatus::Switched, Some(who.user.clone()), Some(who.user.clone())),
        Outcome::Cancelled => unchanged(SyncStatus::Cancelled),
        Outcome::Failed(error) => SyncResult { error: Some(error.clone()), ..unchanged(SyncStatus::Failed) },
    };
    out.saved = saved;
    out
}

/// 文件管理当前以谁的身份运行（徽标、上传是否走 sudo 兜底都看它）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub login_user: String,
    pub file_user: String,
    pub terminal_user: Option<String>,
}

impl FileIdentity {
    /// 已跟随终端切到别的用户。
    pub fn elevated(&self) -> bool {
        self.file_user != self.login_user
    }
}

/// 每个会话的记录。会话被移除（断开 / 远端关闭）时整条丢掉。
#[derive(Clone, Debug, Default)]
pub struct SessionRecord {
    pub identity: Option<FileIdentity>,
    /// 自动同步时用户取消过输密码的终端用户：不再每次切到文件管理都弹框，点徽标才再问。
    pub declined: Option<String>,
    /// 已保存密码自动试过、被服务器拒绝的用户（要谁的密码）。
    pub rejected_saved: HashSet<String>,
    /// 这个会话的文件管理停在哪个远程目录（空 = 还没打开过，第一次从远程 home 开始）。
    pub remote_path: String,
}

/// 一次同步之后该记谁为「已拒绝」：取消了记这位终端用户；切换成功或终端已换了人就清掉；探测失败保持原样。
pub fn next_declined_user(previous: Option<&str>, result: &SyncResult) -> Option<String> {
    match result.status {
        SyncStatus::Cancelled => result.terminal_user.clone(),
        SyncStatus::Switched => None,
        _ => match result.terminal_user.as_deref() {
            None => previous.map(String::from),
            Some(user) if Some(user) == previous => previous.map(String::from),
            Some(_) => None,
        },
    }
}

/// 同步结果写回会话记录。途中会话可能已被移除（断开 / 远端关闭）：那就丢弃结果、什么都不写，
/// 返回 false（调用方也别再刷新列表、弹报错）。
pub fn record_sync_result(
    records: &mut HashMap<String, SessionRecord>,
    session_id: &str,
    result: &SyncResult,
    login_user: &str,
    terminal_user: &str,
    declined_user: Option<&str>,
) -> bool {
    let Some(record) = records.get_mut(session_id) else { return false };
    if let Some(file_user) = &result.file_user {
        record.identity = Some(FileIdentity {
            login_user: login_user.to_string(),
            file_user: file_user.clone(),
            terminal_user: Some(terminal_user.to_string()),
        });
    }
    record.declined = next_declined_user(declined_user, result);
    if let Some((user, ok)) = &result.saved {
        if *ok {
            record.rejected_saved.remove(user);
        } else {
            record.rejected_saved.insert(user.clone());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use futures::executor::block_on;

    use super::*;

    fn wrong() -> Outcome {
        Outcome::Failed("SUDO_PASSWORD_INCORRECT: su: Authentication failure".into())
    }

    fn required() -> Outcome {
        Outcome::Failed("SUDO_PASSWORD_REQUIRED: x".into())
    }

    fn alice() -> TerminalIdentity {
        TerminalIdentity { login_user: "alice".into(), user: "alice".into(), via: None, credential: Some("password".into()), file_user: "alice".into() }
    }

    fn as_user(user: &str, via: Via, credential: Option<&str>) -> TerminalIdentity {
        TerminalIdentity { user: user.into(), via: Some(via), credential: credential.map(String::from), ..alice() }
    }

    fn pw(value: &str) -> Secret {
        Secret::Password(value.into())
    }

    fn cred(value: &str) -> Secret {
        Secret::Credential(value.into())
    }

    #[derive(Default)]
    struct Calls {
        switch: Vec<(Option<SwitchTarget>, Option<Secret>)>,
        ask: Vec<(bool, Vec<String>, PasswordContext)>,
    }

    struct Harness {
        who: Option<TerminalIdentity>,
        switch_results: Vec<Outcome>,
        passwords: Vec<Secret>,
    }

    impl Harness {
        fn new(who: Option<TerminalIdentity>) -> Self {
            Self { who, switch_results: vec![], passwords: vec![] }
        }

        fn switches(mut self, results: Vec<Outcome>) -> Self {
            self.switch_results = results;
            self
        }

        fn passwords(mut self, passwords: Vec<Secret>) -> Self {
            self.passwords = passwords;
            self
        }

        fn run(self, interactive: bool, declined: Option<&str>, rejected: &HashSet<String>) -> (SyncResult, Calls) {
            let calls = RefCell::new(Calls::default());
            let results = RefCell::new(VecDeque::from(self.switch_results));
            let passwords = RefCell::new(VecDeque::from(self.passwords));
            let who = self.who;
            let result = block_on(sync_file_user(
                async || who,
                async |target, secret| {
                    calls.borrow_mut().switch.push((target, secret));
                    results.borrow_mut().pop_front().unwrap_or(Outcome::Ok)
                },
                async |retry, exclude, context| {
                    calls.borrow_mut().ask.push((retry, exclude, context));
                    passwords.borrow_mut().pop_front()
                },
                interactive,
                declined,
                rejected,
            ));
            (result, calls.into_inner())
        }
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    fn target(user: &str, via: Via) -> Option<SwitchTarget> {
        Some(SwitchTarget { user: user.into(), via })
    }

    #[test]
    fn same_user_does_nothing() {
        let (result, calls) = Harness::new(Some(alice())).run(true, None, &none());
        assert_eq!(result, SyncResult::new(SyncStatus::Same, Some("alice".into()), Some("alice".into())));
        assert!(calls.switch.is_empty());
    }

    #[test]
    fn su_with_saved_root_password_switches_without_asking() {
        let (result, calls) = Harness::new(Some(as_user("root", Via::Su, Some("acct:root")))).run(true, None, &none());
        assert_eq!(result.status, SyncStatus::Switched);
        assert_eq!(result.file_user.as_deref(), Some("root"));
        assert_eq!(result.saved, Some(("root".into(), true)));
        assert_eq!(calls.switch, [(target("root", Via::Su), Some(cred("acct:root")))]);
        assert!(calls.ask.is_empty());
    }

    #[test]
    fn without_saved_password_asks_then_switches() {
        let (result, calls) = Harness::new(Some(as_user("root", Via::Su, None)))
            .switches(vec![Outcome::Failed("SUDO_PASSWORD_REQUIRED: 需要 root 的密码".into()), wrong(), Outcome::Ok])
            .passwords(vec![cred("acct:admin"), pw("typed")])
            .run(true, None, &none());
        assert_eq!(result.status, SyncStatus::Switched);
        let secrets: Vec<_> = calls.switch.iter().map(|(_, s)| s.clone()).collect();
        assert_eq!(secrets, [None, Some(cred("acct:admin")), Some(pw("typed"))]);
        let asked: Vec<_> = calls.ask.iter().map(|(retry, exclude, c)| (*retry, exclude.clone(), c.user.clone(), c.via, c.need_user.clone())).collect();
        assert_eq!(
            asked,
            [(false, vec![], "root".into(), Via::Su, "root".into()), (true, vec!["acct:admin".to_string()], "root".into(), Via::Su, "root".into())]
        );
    }

    #[test]
    fn sudo_needs_the_login_users_password() {
        let (result, calls) = Harness::new(Some(as_user("root", Via::Sudo, None)))
            .switches(vec![Outcome::Failed("SUDO_PASSWORD_REQUIRED: a password is required".into()), Outcome::Ok])
            .passwords(vec![pw("alicepw")])
            .run(true, None, &none());
        assert_eq!(result.status, SyncStatus::Switched);
        assert_eq!(calls.ask[0].2.need_user, "alice");
        assert_eq!(calls.switch[1], (target("root", Via::Sudo), Some(pw("alicepw"))));
    }

    #[test]
    fn cancelling_keeps_the_identity() {
        let (result, _) = Harness::new(Some(as_user("root", Via::Su, None))).switches(vec![required()]).run(true, None, &none());
        assert_eq!(result, SyncResult::new(SyncStatus::Cancelled, Some("alice".into()), Some("root".into())));
    }

    #[test]
    fn declined_users_are_not_asked_again_automatically() {
        let who = as_user("root", Via::Su, None);
        let (quiet, calls) = Harness::new(Some(who.clone())).switches(vec![required()]).run(false, Some("root"), &none());
        assert_eq!(quiet.status, SyncStatus::Cancelled);
        assert!(calls.switch.is_empty(), "不弹框也不白跑一次");
        let (asked, _) = Harness::new(Some(who)).switches(vec![required()]).passwords(vec![pw("rootpw")]).run(true, Some("root"), &none());
        assert_eq!(asked.status, SyncStatus::Switched);
    }

    #[test]
    fn declining_only_applies_to_that_user() {
        let mut declined: Option<String> = None;
        let mut sync = |who: TerminalIdentity, interactive: bool| {
            let (result, calls) = Harness::new(Some(who)).switches(vec![required()]).run(interactive, declined.as_deref(), &none());
            declined = next_declined_user(declined.as_deref(), &result);
            calls.ask.iter().map(|(_, _, c)| c.user.clone()).collect::<Vec<_>>()
        };
        let root = as_user("root", Via::Su, None);
        let bob = as_user("bob", Via::Su, None);
        assert_eq!(sync(root.clone(), false), ["root"], "第一次切到 root：询问");
        assert_eq!(sync(root, false), Vec::<String>::new(), "还是 root：不再打扰");
        assert_eq!(sync(bob.clone(), false), ["bob"], "终端换成 bob：照常询问");
        assert_eq!(sync(bob.clone(), false), Vec::<String>::new(), "还是 bob：不再打扰");
        assert_eq!(sync(bob, true), ["bob"], "点徽标：总要问");
    }

    #[test]
    fn how_the_declined_record_carries_over() {
        let r = |status, file: Option<&str>, terminal: Option<&str>| SyncResult::new(status, file.map(String::from), terminal.map(String::from));
        assert_eq!(next_declined_user(None, &r(SyncStatus::Cancelled, Some("alice"), Some("root"))).as_deref(), Some("root"));
        assert_eq!(next_declined_user(Some("root"), &r(SyncStatus::Switched, Some("root"), Some("root"))), None);
        assert_eq!(next_declined_user(Some("root"), &r(SyncStatus::Same, Some("alice"), Some("alice"))), None, "终端 exit 回了 alice");
        assert_eq!(next_declined_user(Some("root"), &r(SyncStatus::Same, None, None)).as_deref(), Some("root"));
        assert_eq!(next_declined_user(Some("root"), &r(SyncStatus::Failed, Some("alice"), Some("root"))).as_deref(), Some("root"));
        assert_eq!(next_declined_user(Some("root"), &r(SyncStatus::Failed, Some("alice"), Some("bob"))), None);
    }

    #[test]
    fn declined_user_with_own_saved_password_still_switches() {
        let (result, calls) = Harness::new(Some(as_user("root", Via::Su, Some("acct:root")))).run(false, Some("root"), &none());
        assert_eq!(result.status, SyncStatus::Switched);
        assert_eq!(calls.switch, [(target("root", Via::Su), Some(cred("acct:root")))]);
        assert!(calls.ask.is_empty());
    }

    #[test]
    fn terminal_exit_switches_back_to_the_login_user() {
        let who = TerminalIdentity { file_user: "root".into(), ..alice() };
        let (result, calls) = Harness::new(Some(who)).run(true, None, &none());
        assert_eq!(result, SyncResult::new(SyncStatus::Switched, Some("alice".into()), Some("alice".into())));
        assert_eq!(calls.switch, [(None, None)]);
    }

    #[test]
    fn missing_sudo_rights_fail_with_the_reason() {
        let (result, _) = Harness::new(Some(as_user("root", Via::Sudo, Some("password"))))
            .switches(vec![Outcome::Failed("SUDO_NOT_ALLOWED: alice is not in the sudoers file.".into())])
            .run(true, None, &none());
        assert_eq!(result.status, SyncStatus::Failed);
        assert!(result.error.unwrap().contains("SUDO_NOT_ALLOWED"));
    }

    #[test]
    fn failed_probe_counts_as_unchanged() {
        let (result, calls) = Harness::new(None).run(true, None, &none());
        assert_eq!(result, SyncResult::new(SyncStatus::Same, None, None));
        assert!(calls.switch.is_empty());
    }

    #[test]
    fn results_for_removed_sessions_are_dropped() {
        let mut records = HashMap::new();
        let cancelled = SyncResult::new(SyncStatus::Cancelled, Some("alice".into()), Some("root".into()));
        assert!(!record_sync_result(&mut records, "gone", &cancelled, "alice", "root", None));
        assert!(records.is_empty(), "会话已不在：不写回身份和拒绝记录");
    }

    #[test]
    fn results_for_live_sessions_are_written_back() {
        let mut records = HashMap::from([("alive".to_string(), SessionRecord::default())]);
        let cancelled = SyncResult::new(SyncStatus::Cancelled, Some("alice".into()), Some("root".into()));
        assert!(record_sync_result(&mut records, "alive", &cancelled, "alice", "root", None));
        let record = &records["alive"];
        assert_eq!(record.identity, Some(FileIdentity { login_user: "alice".into(), file_user: "alice".into(), terminal_user: Some("root".into()) }));
        assert_eq!(record.declined.as_deref(), Some("root"));
        let switched = SyncResult::new(SyncStatus::Switched, Some("root".into()), Some("root".into()));
        assert!(record_sync_result(&mut records, "alive", &switched, "alice", "root", Some("root")));
        assert_eq!(records["alive"].identity.as_ref().unwrap().file_user, "root");
        assert_eq!(records["alive"].declined, None);
    }

    #[test]
    fn rejected_saved_passwords_are_not_retried_automatically() {
        let id = "saved-bad";
        let mut records = HashMap::from([(id.to_string(), SessionRecord::default())]);
        let who = as_user("root", Via::Su, Some("acct:root"));
        let sync = |records: &mut HashMap<String, SessionRecord>, interactive: bool, switches: Vec<Outcome>, passwords: Vec<Secret>| {
            let record = records[id].clone();
            let (result, calls) = Harness::new(Some(who.clone()))
                .switches(switches)
                .passwords(passwords)
                .run(interactive, record.declined.as_deref(), &record.rejected_saved);
            record_sync_result(records, id, &result, "alice", "root", record.declined.as_deref());
            let tried: Vec<_> = calls.switch.iter().map(|(_, s)| s.clone()).collect();
            let asked: Vec<_> = calls.ask.iter().map(|(_, e, _)| e.clone()).collect();
            (result, tried, asked)
        };

        let (_, tried, asked) = sync(&mut records, false, vec![wrong()], vec![]);
        assert_eq!(tried, [Some(cred("acct:root"))], "第一次：自动用 root 自己的已保存密码");
        assert_eq!(asked, [vec!["acct:root".to_string()]], "服务器说不对：弹框，不列这个错密码；用户取消");
        assert!(records[id].rejected_saved.contains("root"));

        let (_, tried, asked) = sync(&mut records, false, vec![], vec![]);
        assert!(tried.is_empty(), "再次自动同步：不再拿错密码去试");
        assert!(asked.is_empty(), "用户拒绝过 root：也不打扰");

        records.get_mut(id).unwrap().declined = None;
        let (result, tried, asked) = sync(&mut records, false, vec![required(), Outcome::Ok], vec![pw("rootpw")]);
        assert_eq!(tried, [None, Some(pw("rootpw"))], "不先试那个错密码");
        assert_eq!(asked, [vec!["acct:root".to_string()]]);
        assert_eq!(result.status, SyncStatus::Switched);
        assert!(records[id].rejected_saved.contains("root"), "手输的对了，不代表已保存的那个对");

        let (_, tried, _) = sync(&mut records, true, vec![Outcome::Ok], vec![]);
        assert_eq!(tried, [Some(cred("acct:root"))], "点徽标：照旧先试已保存密码");
        assert!(!records[id].rejected_saved.contains("root"), "这次对了：撤销记录");
    }

    #[test]
    fn rejected_saved_record_is_per_user() {
        let rejected = HashSet::from(["root".to_string()]);
        // sudo 要的是登录用户 alice 的密码：root 的记录不影响它
        let (result, calls) = Harness::new(Some(as_user("root", Via::Sudo, Some("password")))).run(false, None, &rejected);
        assert_eq!(result.status, SyncStatus::Switched);
        let tried: Vec<_> = calls.switch.iter().map(|(_, s)| s.clone()).collect();
        assert_eq!(tried, [Some(cred("password"))]);
    }
}

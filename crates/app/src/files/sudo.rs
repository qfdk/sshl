//! 上传到登录用户无权写入的目录时的 sudo 兜底（旧 sudo-upload.ts）。
//! 普通上传报 Permission denied → 问是否用 sudo → 后端 file_upload_sudo（暂存 + sudo 以目录属主身份解包）。
//! sudo 只认登录用户自己的密码：先用他自己的已保存密码，不拿别的已保存密码挨个试（试错会进服务器认证日志、可能锁号）。
//! 终端切换了用户时文件管理整个以那个用户身份运行（见 identity.rs），不走这里的兜底。

use sshl_core::config_store::CredList;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// 用户在确认框或密码框里取消：不算失败，不弹报错。
    Cancelled,
    Failed(String),
}

impl Outcome {
    pub fn from_result<T, E: std::fmt::Display>(result: Option<Result<T, E>>) -> Self {
        match result {
            Some(Ok(_)) => Outcome::Ok,
            Some(Err(error)) => Outcome::Failed(error.to_string()),
            None => Outcome::Failed("后台任务异常退出".into()),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Outcome::Failed(error) => Some(error),
            _ => None,
        }
    }
}

/// 手动输入的密码，或已保存密码的标识（"password" = 连接登录密码，"acct:<账号>"）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Secret {
    Password(String),
    Credential(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SudoFailure {
    PasswordRequired,
    PasswordIncorrect,
    NotAllowed,
    Other,
}

const MAX_PASSWORD_ATTEMPTS: usize = 3;

pub fn is_permission_denied(error: Option<&str>) -> bool {
    error.is_some_and(|e| e.to_lowercase().contains("permission denied"))
}

pub fn sudo_failure(error: Option<&str>) -> SudoFailure {
    let text = error.unwrap_or("");
    if text.contains("SUDO_PASSWORD_INCORRECT") {
        SudoFailure::PasswordIncorrect
    } else if text.contains("SUDO_PASSWORD_REQUIRED") {
        SudoFailure::PasswordRequired
    } else if text.contains("SUDO_NOT_ALLOWED") {
        SudoFailure::NotAllowed
    } else {
        SudoFailure::Other
    }
}

/// 去掉开头的 `SUDO_XXX:` 标记。
fn strip_marker(error: &str) -> &str {
    let Some(rest) = error.strip_prefix("SUDO_") else { return error };
    let marker_end = rest.find(|c: char| !(c.is_ascii_uppercase() || c == '_')).unwrap_or(rest.len());
    match rest[marker_end..].strip_prefix(':') {
        Some(after) => after.trim_start(),
        None => error,
    }
}

/// 给报错对话框用的人话；原始信息附在后面便于排查。
pub fn sudo_error_message(error: Option<&str>) -> String {
    let full = error.unwrap_or("");
    let raw = strip_marker(full);
    match sudo_failure(error) {
        SudoFailure::NotAllowed => format!("当前登录用户没有 sudo 权限。\n{raw}"),
        SudoFailure::PasswordIncorrect => format!("sudo 密码不正确。\n{raw}"),
        _ if full.contains("SUDO_TTY_REQUIRED") => format!("服务器的 sudo 要求终端（requiretty），无法在后台执行。\n{raw}"),
        _ if raw.is_empty() => "未知错误".into(),
        _ => raw.to_string(),
    }
}

/// 弹框里可点选的已保存密码 (标识, 显示名)：要谁的密码（need_user），他名下的排前面；连接密码属于登录用户；已试错的不列。
pub fn saved_password_choices(saved: Option<&CredList>, need_user: &str, exclude: &[String], login_user: &str) -> Vec<(String, String)> {
    let mut all: Vec<(String, String, String)> = Vec::new();
    if let Some(saved) = saved {
        if saved.has_password {
            all.push(("password".into(), "连接密码".into(), login_user.to_string()));
        }
        for account in &saved.accounts {
            all.push((format!("acct:{account}"), account.clone(), account.clone()));
        }
    }
    let (mine, others): (Vec<_>, Vec<_>) = all.into_iter().partition(|(_, _, owner)| owner == need_user);
    mine.into_iter()
        .chain(others)
        .filter(|(value, _, _)| !exclude.contains(value))
        .map(|(value, label, _)| (value, label))
        .collect()
}

/// 先用当前要的那位用户自己的已保存密码（own，没有就不带密码），要密码 / 密码错再弹框，最多再问三次。
pub async fn retry_with_password(
    mut run: impl AsyncFnMut(Option<Secret>) -> Outcome,
    own: Option<String>,
    mut ask: impl AsyncFnMut(bool, Vec<String>) -> Option<Secret>,
) -> Outcome {
    let mut failed: Vec<String> = Vec::new();
    let mut secret = own.map(Secret::Credential);
    let mut result = run(secret.clone()).await;
    for _ in 0..MAX_PASSWORD_ATTEMPTS {
        if result == Outcome::Ok {
            break;
        }
        let failure = sudo_failure(result.error());
        if failure != SudoFailure::PasswordRequired && failure != SudoFailure::PasswordIncorrect {
            break;
        }
        if failure == SudoFailure::PasswordIncorrect
            && let Some(Secret::Credential(tried)) = &secret
        {
            failed.push(tried.clone());
        }
        secret = ask(failure == SudoFailure::PasswordIncorrect, failed.clone()).await;
        if secret.is_none() {
            return Outcome::Cancelled;
        }
        result = run(secret.clone()).await;
    }
    result
}

/// `elevated`：文件管理已跟随终端切到别的用户——权限问题就是那个用户的，不再用登录用户的 sudo 兜底。
pub async fn upload_with_sudo_fallback(
    normal: impl AsyncFnOnce() -> Outcome,
    sudo: impl AsyncFnMut(Option<Secret>) -> Outcome,
    confirm: impl AsyncFnOnce() -> bool,
    own_password: impl AsyncFnOnce() -> Option<String>,
    ask: impl AsyncFnMut(bool, Vec<String>) -> Option<Secret>,
    elevated: bool,
) -> Outcome {
    let first = normal().await;
    if elevated || first == Outcome::Ok || !is_permission_denied(first.error()) {
        return first;
    }
    if !confirm().await {
        return Outcome::Cancelled;
    }
    let own = own_password().await;
    retry_with_password(sudo, own, ask).await
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use futures::executor::block_on;

    use super::*;

    const DENIED: &str = "SFTP 错误: open remote /home/bob/a.txt: Permission denied: Permission denied";

    fn denied() -> Outcome {
        Outcome::Failed(DENIED.into())
    }

    fn wrong() -> Outcome {
        Outcome::Failed("SUDO_PASSWORD_INCORRECT: Sorry, try again.".into())
    }

    fn pw(value: &str) -> Secret {
        Secret::Password(value.into())
    }

    fn cred(value: &str) -> Secret {
        Secret::Credential(value.into())
    }

    #[derive(Default)]
    struct Calls {
        sudo: Vec<Option<Secret>>,
        confirm: usize,
        ask: Vec<bool>,
        exclude: Vec<Vec<String>>,
    }

    fn run(normal: Outcome, sudo_results: Vec<Outcome>, confirm: bool, passwords: Vec<Secret>, own: Option<&str>, elevated: bool) -> (Outcome, Calls) {
        let calls = RefCell::new(Calls::default());
        let results = RefCell::new(VecDeque::from(sudo_results));
        let passwords = RefCell::new(VecDeque::from(passwords));
        let outcome = block_on(upload_with_sudo_fallback(
            async || normal,
            async |secret| {
                calls.borrow_mut().sudo.push(secret);
                results.borrow_mut().pop_front().expect("unexpected sudo call")
            },
            async || {
                calls.borrow_mut().confirm += 1;
                confirm
            },
            async || own.map(String::from),
            async |retry, exclude| {
                calls.borrow_mut().ask.push(retry);
                calls.borrow_mut().exclude.push(exclude);
                passwords.borrow_mut().pop_front()
            },
            elevated,
        ));
        (outcome, calls.into_inner())
    }

    #[test]
    fn recognizes_sftp_permission_denied_only() {
        assert!(is_permission_denied(Some(DENIED)));
        assert!(!is_permission_denied(Some("SFTP 错误: open remote /x: No such file: No such file")));
        assert!(!is_permission_denied(None));
    }

    #[test]
    fn classifies_backend_sudo_markers() {
        assert_eq!(sudo_failure(Some("SUDO_PASSWORD_REQUIRED: sudo: a password is required")), SudoFailure::PasswordRequired);
        assert_eq!(sudo_failure(Some("SUDO_PASSWORD_INCORRECT: Sorry, try again.")), SudoFailure::PasswordIncorrect);
        assert_eq!(sudo_failure(Some("SUDO_NOT_ALLOWED: alice is not in the sudoers file.")), SudoFailure::NotAllowed);
        assert_eq!(sudo_failure(Some("SUDO_FAILED: tar: boom")), SudoFailure::Other);
    }

    #[test]
    fn readable_sudo_errors() {
        assert_eq!(sudo_error_message(Some("SUDO_NOT_ALLOWED: alice is not in the sudoers file.")), "当前登录用户没有 sudo 权限。\nalice is not in the sudoers file.");
        assert_eq!(sudo_error_message(Some("SUDO_PASSWORD_INCORRECT: Sorry")), "sudo 密码不正确。\nSorry");
        assert!(sudo_error_message(Some("SUDO_TTY_REQUIRED: sorry, you must have a tty")).starts_with("服务器的 sudo 要求终端"));
        assert_eq!(sudo_error_message(Some("SUDO_FAILED: tar: boom")), "tar: boom");
        assert_eq!(sudo_error_message(None), "未知错误");
    }

    #[test]
    fn normal_upload_success_never_asks() {
        let (outcome, calls) = run(Outcome::Ok, vec![], true, vec![], None, false);
        assert_eq!(outcome, Outcome::Ok);
        assert_eq!(calls.confirm, 0);
        assert!(calls.sudo.is_empty());
    }

    #[test]
    fn other_errors_are_returned_without_mentioning_sudo() {
        let err = Outcome::Failed("SFTP 错误: open remote /x: No such file".into());
        let (outcome, calls) = run(err.clone(), vec![], true, vec![], None, false);
        assert_eq!(outcome, err);
        assert_eq!(calls.confirm, 0);
    }

    #[test]
    fn declining_sudo_is_a_cancel() {
        let (outcome, calls) = run(denied(), vec![], false, vec![], None, false);
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(calls.sudo.is_empty());
    }

    #[test]
    fn passwordless_sudo_first_then_ask_when_required() {
        let (outcome, calls) = run(
            denied(),
            vec![Outcome::Failed("SUDO_PASSWORD_REQUIRED: a password is required".into()), Outcome::Ok],
            true,
            vec![pw("pw1")],
            None,
            false,
        );
        assert_eq!(outcome, Outcome::Ok);
        assert_eq!(calls.sudo, [None, Some(pw("pw1"))]);
        assert_eq!(calls.ask, [false]);
        assert_eq!(calls.exclude, [Vec::<String>::new()]);
    }

    #[test]
    fn own_saved_password_is_used_without_asking() {
        let (outcome, calls) = run(denied(), vec![Outcome::Ok], true, vec![], Some("acct:alice"), false);
        assert_eq!(outcome, Outcome::Ok);
        assert_eq!(calls.sudo, [Some(cred("acct:alice"))]);
        assert!(calls.ask.is_empty());
    }

    #[test]
    fn wrong_saved_passwords_are_not_listed_again() {
        let (outcome, calls) = run(denied(), vec![wrong(), wrong(), Outcome::Ok], true, vec![cred("acct:admin"), pw("typed")], Some("password"), false);
        assert_eq!(outcome, Outcome::Ok);
        // 自动只用了当前用户自己那一份；其余都是用户在弹框里亲手选/输的
        assert_eq!(calls.sudo, [Some(cred("password")), Some(cred("acct:admin")), Some(pw("typed"))]);
        assert_eq!(calls.exclude, [vec!["password".to_string()], vec!["password".to_string(), "acct:admin".to_string()]]);
    }

    #[test]
    fn three_wrong_passwords_return_the_last_error() {
        let typed = vec![pw("a"), pw("b"), pw("c")];
        let (outcome, calls) = run(denied(), vec![wrong(), wrong(), wrong(), wrong()], true, typed.clone(), None, false);
        assert_eq!(outcome, wrong());
        assert_eq!(calls.sudo, [None, Some(pw("a")), Some(pw("b")), Some(pw("c"))]);
        assert_eq!(calls.ask, [true, true, true]);
    }

    #[test]
    fn cancelling_the_password_prompt_is_a_cancel() {
        let (outcome, _) = run(denied(), vec![Outcome::Failed("SUDO_PASSWORD_REQUIRED: x".into())], true, vec![], None, false);
        assert_eq!(outcome, Outcome::Cancelled);
    }

    #[test]
    fn no_sudo_rights_returns_the_error_without_asking() {
        let err = Outcome::Failed("SUDO_NOT_ALLOWED: alice is not in the sudoers file.".into());
        let (outcome, calls) = run(denied(), vec![err.clone()], true, vec![], None, false);
        assert_eq!(outcome, err);
        assert!(calls.ask.is_empty());
    }

    #[test]
    fn elevated_file_manager_skips_the_sudo_fallback() {
        let (outcome, calls) = run(denied(), vec![], true, vec![], None, true);
        assert_eq!(outcome, denied());
        assert_eq!(calls.confirm, 0);
    }

    fn saved(has_password: bool, accounts: &[&str]) -> CredList {
        CredList { has_password, accounts: accounts.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn own_saved_passwords_first_and_wrong_ones_hidden() {
        let saved = saved(true, &["root", "alice", "deploy"]);
        let values = |choices: Vec<(String, String)>| choices.into_iter().map(|(v, _)| v).collect::<Vec<_>>();
        assert_eq!(
            saved_password_choices(Some(&saved), "alice", &[], "alice"),
            [
                ("password".to_string(), "连接密码".to_string()),
                ("acct:alice".to_string(), "alice".to_string()),
                ("acct:root".to_string(), "root".to_string()),
                ("acct:deploy".to_string(), "deploy".to_string()),
            ]
        );
        assert_eq!(values(saved_password_choices(Some(&saved), "alice", &["password".into(), "acct:root".into()], "alice")), ["acct:alice", "acct:deploy"]);
        assert!(saved_password_choices(None, "alice", &[], "alice").is_empty());
    }

    #[test]
    fn switching_to_another_user_puts_his_password_first() {
        let saved = saved(true, &["deploy", "root"]);
        let values: Vec<_> = saved_password_choices(Some(&saved), "root", &[], "alice").into_iter().map(|(v, _)| v).collect();
        assert_eq!(values, ["acct:root", "password", "acct:deploy"]);
    }
}

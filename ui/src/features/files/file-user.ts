// 文件管理跟随终端用户：终端里 su / sudo -i 成了 root，文件管理就以 root 身份浏览、上传、下载、删除。
// 后端 file_terminal_user 从这条连接的进程链看出终端此刻是谁、怎么切过去的；file_switch_user 以他的身份
// 起 sftp-server 换掉 SFTP 会话。密码：su 要目标用户的，sudo 要登录用户的；先用那位用户自己的已保存密码，
// 没有再弹框（retryWithPassword），不拿别的已保存密码去试。
import sessionManager from '../../lib/session-manager';
import { retryWithPassword, sudoFailure, type SudoSecret, type UploadResult } from './sudo-upload';

export type Via = 'su' | 'sudo';
export type TerminalIdentity = { loginUser: string; user: string; via: Via | null; credential: string | null; fileUser: string };
export type SwitchTarget = { user: string; via: Via } | null;
export type PasswordContext = { user: string; via: Via; needUser: string; loginUser: string };
export type SyncResult = {
  status: 'same' | 'switched' | 'cancelled' | 'failed';
  fileUser: string | null;
  terminalUser: string | null;
  error?: string;
  /** 这次自动试了谁的已保存密码、服务器认不认（没试或没得出结论时没有）。 */
  saved?: { user: string; ok: boolean };
};

export async function syncFileUser({ identify, switchUser, askPassword, interactive = true, declinedUser = null, rejectedSaved }: {
  identify: () => Promise<TerminalIdentity | null>;
  switchUser: (target: SwitchTarget, secret: SudoSecret | null) => Promise<UploadResult>;
  askPassword: (retry: boolean, exclude: string[], context: PasswordContext) => Promise<SudoSecret | null>;
  /** true：点徽标，总要问。false：自动同步（切到文件管理 / 刷新时）。 */
  interactive?: boolean;
  /** 自动同步时用户取消过输密码的那位终端用户：没有可自动用的密码（没保存或已知不对）就不再弹框打扰；终端换成别人照常询问。 */
  declinedUser?: string | null;
  /** 这个会话里已保存密码自动试过、被服务器拒绝的用户（按要谁的密码记）：自动同步不再拿它去试，点徽标时照旧先试。 */
  rejectedSaved?: ReadonlySet<string>;
}): Promise<SyncResult> {
  const who = await identify();
  if (!who) return { status: 'same', fileUser: null, terminalUser: null };
  const unchanged = { fileUser: who.fileUser, terminalUser: who.user };
  if (who.user === who.fileUser) return { status: 'same', ...unchanged };
  if (!who.via) {
    // 终端 exit 回到了登录用户
    const back = await switchUser(null, null);
    return back?.success ? { status: 'switched', fileUser: who.loginUser, terminalUser: who.user } : { status: 'failed', ...unchanged, error: back?.error };
  }
  const needUser = who.via === 'su' ? who.user : who.loginUser;
  // 已知不对的已保存密码，自动同步不再去试：每次失败的 su / sudo 都进服务器认证日志，可能触发锁号
  const skipped = !interactive && rejectedSaved?.has(needUser) ? who.credential : null;
  const own = skipped ? null : who.credential;
  if (!interactive && who.user === declinedUser && !own) return { status: 'cancelled', ...unchanged };
  const target = { user: who.user, via: who.via };
  const context = { user: who.user, via: who.via, needUser, loginUser: who.loginUser };
  let saved: SyncResult['saved'];
  let first = true;
  const result = await retryWithPassword(
    async secret => {
      const attempt = await switchUser(target, secret);
      // 第一次是自动用他自己的已保存密码（有的话）：记下服务器认不认它
      if (first && own) {
        if (attempt?.success) saved = { user: needUser, ok: true };
        else if (sudoFailure(attempt?.error) === 'password-incorrect') saved = { user: needUser, ok: false };
      }
      first = false;
      return attempt;
    },
    own,
    // 跳过的那个错密码也不列进弹框的可选项（同一次里试错的本来就不列）
    (retry, exclude) => askPassword(retry, skipped && !exclude.includes(skipped) ? [skipped, ...exclude] : exclude, context),
  );
  const outcome = saved ? { saved } : {};
  if (result?.success) return { status: 'switched', fileUser: who.user, terminalUser: who.user, ...outcome };
  if (result?.cancelled) return { status: 'cancelled', ...unchanged, ...outcome };
  return { status: 'failed', ...unchanged, error: result?.error, ...outcome };
}

// 每个会话文件管理当前以谁的身份运行（徽标显示、上传是否走 sudo 兜底都看它）。
export type FileIdentity = { loginUser: string; fileUser: string; terminalUser?: string };
const identities = new Map<string, FileIdentity>();
const listeners = new Set<() => void>();

export function getFileIdentity(sessionId: string | null) {
  return (sessionId && identities.get(sessionId)) || null;
}

export function setFileIdentity(sessionId: string, identity: FileIdentity | null) {
  if (identity) identities.set(sessionId, identity);
  else identities.delete(sessionId);
  listeners.forEach(listener => listener());
}

export function subscribeFileIdentity(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

// 自动同步时用户取消过输密码的（会话 → 当时的终端用户）：不再每次切到文件管理都弹框，点徽标才再问。
const declined = new Map<string, string>();

export function getDeclinedUser(sessionId: string) {
  return declined.get(sessionId) ?? null;
}

export function setDeclinedUser(sessionId: string, user: string | null) {
  if (user) declined.set(sessionId, user);
  else declined.delete(sessionId);
}

/** 一次同步之后该记谁为「已拒绝」：取消了记这位终端用户；切换成功或终端已换了人就清掉；探测失败保持原样。 */
export function nextDeclinedUser(previous: string | null, result: SyncResult) {
  if (result.status === 'cancelled') return result.terminalUser;
  if (result.status === 'switched') return null;
  return result.terminalUser === null || result.terminalUser === previous ? previous : null;
}

/**
 * 一次同步的结果写回这个会话的记录（文件管理身份、拒绝记录）。同步要等探测、等用户输密码，途中会话可能
 * 已被移除（断开 / 远端关闭）：那就丢弃结果、什么都不写，否则 onSessionRemoved 清过的记录又被写回来。
 * 返回是否写回（false = 会话已不在，调用方也别再刷新列表、弹报错）。
 */
export function recordSyncResult(sessionId: string, result: SyncResult, { loginUser, terminalUser, declinedUser }: {
  loginUser: string;
  terminalUser: string;
  /** 同步开始时读到的拒绝记录。 */
  declinedUser: string | null;
}) {
  if (!sessionManager.getSession(sessionId)) return false;
  if (result.fileUser) setFileIdentity(sessionId, { loginUser, fileUser: result.fileUser, terminalUser });
  setDeclinedUser(sessionId, nextDeclinedUser(declinedUser, result));
  if (result.saved) markSavedResult(sessionId, result.saved.user, result.saved.ok);
  return true;
}

// 这个会话里已保存密码自动试过、被服务器拒绝的用户（会话 → 要谁的密码）
const rejectedSaved = new Map<string, Set<string>>();
const NONE: ReadonlySet<string> = new Set();

export function getRejectedSaved(sessionId: string): ReadonlySet<string> {
  return rejectedSaved.get(sessionId) ?? NONE;
}

function markSavedResult(sessionId: string, user: string, ok: boolean) {
  const users = rejectedSaved.get(sessionId);
  if (!ok) {
    if (users) users.add(user);
    else rejectedSaved.set(sessionId, new Set([user]));
  } else if (users?.delete(user) && !users.size) {
    rejectedSaved.delete(sessionId);
  }
}

// 会话没了（断开 / 远端关闭）就丢掉它的身份、取消记录和已保存密码被拒的记录，否则按 sessionId 只增不删
sessionManager.onSessionRemoved(sessionId => {
  setFileIdentity(sessionId, null);
  declined.delete(sessionId);
  rejectedSaved.delete(sessionId);
});

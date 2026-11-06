// 文件管理跟随终端用户：终端里 su / sudo -i 成了 root，文件管理就以 root 身份浏览、上传、下载、删除。
// 后端 file_terminal_user 从这条连接的进程链看出终端此刻是谁、怎么切过去的；file_switch_user 以他的身份
// 起 sftp-server 换掉 SFTP 会话。密码：su 要目标用户的，sudo 要登录用户的；先用那位用户自己的已保存密码，
// 没有再弹框（retryWithPassword），不拿别的已保存密码去试。
import sessionManager from '../../lib/session-manager';
import { retryWithPassword, type SudoSecret, type UploadResult } from './sudo-upload';

export type Via = 'su' | 'sudo';
export type TerminalIdentity = { loginUser: string; user: string; via: Via | null; credential: string | null; fileUser: string };
export type SwitchTarget = { user: string; via: Via } | null;
export type PasswordContext = { user: string; via: Via; needUser: string; loginUser: string };
export type SyncResult = { status: 'same' | 'switched' | 'cancelled' | 'failed'; fileUser: string | null; terminalUser: string | null; error?: string };

export async function syncFileUser({ identify, switchUser, askPassword, prompt = true }: {
  identify: () => Promise<TerminalIdentity | null>;
  switchUser: (target: SwitchTarget, secret: SudoSecret | null) => Promise<UploadResult>;
  askPassword: (retry: boolean, exclude: string[], context: PasswordContext) => Promise<SudoSecret | null>;
  /** false：自动同步（切标签页时），没保存密码就不弹框打扰；点徽标时为 true。 */
  prompt?: boolean;
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
  if (!prompt && !who.credential) return { status: 'cancelled', ...unchanged };
  const target = { user: who.user, via: who.via };
  const context = { user: who.user, via: who.via, needUser: who.via === 'su' ? who.user : who.loginUser, loginUser: who.loginUser };
  const result = await retryWithPassword(
    secret => switchUser(target, secret),
    who.credential,
    (retry, exclude) => askPassword(retry, exclude, context),
  );
  if (result?.success) return { status: 'switched', fileUser: who.user, terminalUser: who.user };
  if (result?.cancelled) return { status: 'cancelled', ...unchanged };
  return { status: 'failed', ...unchanged, error: result?.error };
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

// 会话没了（断开 / 远端关闭）就丢掉它的身份和取消记录，否则按 sessionId 只增不删
sessionManager.onSessionRemoved(sessionId => {
  setFileIdentity(sessionId, null);
  declined.delete(sessionId);
});

// 上传到登录用户无权写入的目录（如别的用户的家目录）时的 sudo 兜底。
// 普通 SFTP 上传报 Permission denied → 问用户是否用 sudo → 后端 file_upload_sudo（暂存 + sudo 以目录属主身份解包）。
// sudo 只认登录用户自己的密码：先问后端当前用户是谁、有没有保存他自己的密码，有就只用这一份，
// 没有走免密 sudo。不拿别的已保存密码挨个试（试错会记进服务器认证日志、可能触发锁定）。
// 不行再弹框：用户可点选已保存的密码（只传标识，后端解密，不经过界面）或手动输入；
// 试错的已保存密码不再列出，最多再问三次。

export type UploadResult = { success: boolean; error?: string; cancelled?: boolean } | null | undefined;
export type SudoFailure = 'password-required' | 'password-incorrect' | 'not-allowed' | 'other';
/** 手动输入的密码，或已保存密码的标识（'password' = 连接登录密码，'acct:<账号>'）。 */
export type SudoSecret = { password: string } | { credential: string };

const MAX_PASSWORD_ATTEMPTS = 3;

export function isPermissionDenied(error: string | undefined) {
  return /permission denied/i.test(error || '');
}

export function sudoFailure(error: string | undefined): SudoFailure {
  const text = error || '';
  if (text.includes('SUDO_PASSWORD_INCORRECT')) return 'password-incorrect';
  if (text.includes('SUDO_PASSWORD_REQUIRED')) return 'password-required';
  if (text.includes('SUDO_NOT_ALLOWED')) return 'not-allowed';
  return 'other';
}

/** 给报错对话框用的人话；原始信息附在后面便于排查。 */
export function sudoErrorMessage(error: string | undefined) {
  const raw = (error || '').replace(/^SUDO_[A-Z_]+:\s*/, '');
  switch (sudoFailure(error)) {
    case 'not-allowed': return `当前登录用户没有 sudo 权限。\n${raw}`;
    case 'password-incorrect': return `sudo 密码不正确。\n${raw}`;
    default: return (error || '').includes('SUDO_TTY_REQUIRED')
      ? `服务器的 sudo 要求终端（requiretty），无法在后台执行。\n${raw}`
      : raw || '未知错误';
  }
}

/** 弹框里可点选的已保存密码：连接密码、当前用户名下的账号密码排前面，已试错的不列。 */
export function savedPasswordChoices(saved: { hasPassword?: boolean; accounts?: string[] } | null | undefined, user: string, exclude: string[]) {
  const accounts = [...(saved?.accounts || [])].sort((a, b) => Number(b === user) - Number(a === user));
  return [
    ...(saved?.hasPassword ? [{ value: 'password', label: '连接密码' }] : []),
    ...accounts.map(account => ({ value: `acct:${account}`, label: account })),
  ].filter(choice => !exclude.includes(choice.value));
}

export async function uploadWithSudoFallback({ normal, sudo, confirm, ownPassword, askPassword }: {
  normal: () => Promise<UploadResult>;
  sudo: (secret: SudoSecret | null) => Promise<UploadResult>;
  confirm: () => Promise<boolean>;
  /** 当前登录用户自己的已保存密码标识（没有为 null）。 */
  ownPassword: () => Promise<string | null>;
  /** exclude：已试过且不对的已保存密码标识，弹框里不再列出。 */
  askPassword: (retry: boolean, exclude: string[]) => Promise<SudoSecret | null>;
}): Promise<UploadResult> {
  const first = await normal();
  if (first?.success || !isPermissionDenied(first?.error)) return first;
  if (!await confirm()) return { success: false, cancelled: true };

  const failed: string[] = [];
  const own = await ownPassword();
  let secret: SudoSecret | null = own ? { credential: own } : null;
  let result = await sudo(secret);
  for (let attempt = 0; attempt < MAX_PASSWORD_ATTEMPTS && !result?.success; attempt++) {
    const failure = sudoFailure(result?.error);
    if (failure !== 'password-required' && failure !== 'password-incorrect') break;
    const tried = secret && 'credential' in secret ? secret.credential : null;
    if (failure === 'password-incorrect' && tried) failed.push(tried);
    secret = await askPassword(failure === 'password-incorrect', failed);
    if (secret === null) return { success: false, cancelled: true };
    result = await sudo(secret);
  }
  return result;
}

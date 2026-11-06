// 上传到登录用户无权写入的目录（如别的用户的家目录）时的 sudo 兜底。
// 文件管理以登录用户身份运行时（终端没切换用户），普通上传报 Permission denied → 问是否用 sudo →
// 后端 file_upload_sudo（暂存 + sudo 以目录属主身份解包）。sudo 只认登录用户自己的密码（retryWithPassword）。
// 终端切换了用户时文件管理整个以那个用户身份运行（见 file-user.ts），不走这里的兜底。

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

/** 弹框里可点选的已保存密码：要谁的密码（needUser），他名下的排前面；连接密码属于登录用户；已试错的不列。 */
export function savedPasswordChoices(saved: { hasPassword?: boolean; accounts?: string[] } | null | undefined, needUser: string, exclude: string[], loginUser = needUser) {
  const all = [
    ...(saved?.hasPassword ? [{ value: 'password', label: '连接密码', owner: loginUser }] : []),
    ...(saved?.accounts || []).map(account => ({ value: `acct:${account}`, label: account, owner: account })),
  ];
  return [...all.filter(c => c.owner === needUser), ...all.filter(c => c.owner !== needUser)]
    .filter(choice => !exclude.includes(choice.value))
    .map(({ value, label }) => ({ value, label }));
}

/**
 * 先用当前要的那位用户自己的已保存密码（own，没有就不带密码），要密码 / 密码错再弹框，最多再问三次。
 * 不拿别的已保存密码挨个试：试错会记进服务器认证日志、可能触发锁定。
 */
export async function retryWithPassword(
  run: (secret: SudoSecret | null) => Promise<UploadResult>,
  own: string | null,
  askPassword: (retry: boolean, exclude: string[]) => Promise<SudoSecret | null>,
): Promise<UploadResult> {
  const failed: string[] = [];
  let secret: SudoSecret | null = own ? { credential: own } : null;
  let result = await run(secret);
  for (let attempt = 0; attempt < MAX_PASSWORD_ATTEMPTS && !result?.success; attempt++) {
    const failure = sudoFailure(result?.error);
    if (failure !== 'password-required' && failure !== 'password-incorrect') break;
    const tried = secret && 'credential' in secret ? secret.credential : null;
    if (failure === 'password-incorrect' && tried) failed.push(tried);
    secret = await askPassword(failure === 'password-incorrect', [...failed]);
    if (secret === null) return { success: false, cancelled: true };
    result = await run(secret);
  }
  return result;
}

export async function uploadWithSudoFallback({ normal, sudo, confirm, ownPassword, askPassword, elevated = false }: {
  normal: () => Promise<UploadResult>;
  sudo: (secret: SudoSecret | null) => Promise<UploadResult>;
  confirm: () => Promise<boolean>;
  /** 登录用户自己的已保存密码标识（没有为 null）。 */
  ownPassword: () => Promise<string | null>;
  /** exclude：已试过且不对的已保存密码标识，弹框里不再列出。 */
  askPassword: (retry: boolean, exclude: string[]) => Promise<SudoSecret | null>;
  /** 文件管理已跟随终端切到别的用户：权限问题就是那个用户的，不再用登录用户的 sudo 兜底。 */
  elevated?: boolean;
}): Promise<UploadResult> {
  const first = await normal();
  if (elevated || first?.success || !isPermissionDenied(first?.error)) return first;
  if (!await confirm()) return { success: false, cancelled: true };
  return retryWithPassword(sudo, await ownPassword(), askPassword);
}

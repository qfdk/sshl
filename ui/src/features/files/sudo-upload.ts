// 上传到登录用户无权写入的目录（如别的用户的家目录）时的 sudo 兜底。
// 普通 SFTP 上传报 Permission denied → 问用户是否用 sudo → 后端 file_upload_sudo（暂存 + sudo 以目录属主身份解包）。
// sudo 密码先用连接保存的登录密码或免密 sudo（后端自己取，前端传 null），不行再弹框要，密码错最多再问三次。

export type UploadResult = { success: boolean; error?: string; cancelled?: boolean } | null | undefined;
export type SudoFailure = 'password-required' | 'password-incorrect' | 'not-allowed' | 'other';

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

export async function uploadWithSudoFallback({ normal, sudo, confirm, askPassword }: {
  normal: () => Promise<UploadResult>;
  sudo: (password: string | null) => Promise<UploadResult>;
  confirm: () => Promise<boolean>;
  askPassword: (retry: boolean) => Promise<string | null>;
}): Promise<UploadResult> {
  const first = await normal();
  if (first?.success || !isPermissionDenied(first?.error)) return first;
  if (!await confirm()) return { success: false, cancelled: true };

  let result = await sudo(null);
  for (let attempt = 0; attempt < MAX_PASSWORD_ATTEMPTS && !result?.success; attempt++) {
    const failure = sudoFailure(result?.error);
    if (failure !== 'password-required' && failure !== 'password-incorrect') break;
    const password = await askPassword(failure === 'password-incorrect');
    if (password === null) return { success: false, cancelled: true };
    result = await sudo(password);
  }
  return result;
}

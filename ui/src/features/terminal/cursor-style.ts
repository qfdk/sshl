// 终端默认光标：闪烁竖线。程序用 DECSCUSR 改过光标后，「恢复默认」就回到这里。
export const DEFAULT_CURSOR = { cursorStyle: 'bar', cursorBlink: true } as const;

// DECSCUSR（CSI Ps SP q）的 Ps 为 0 或省略 = 恢复终端默认光标。xterm 5.3 把 0 当成 1（闪烁方块），
// nvim 退出时正是发 ESC[0 q，光标就一直停在方块。1–6 是明确的形状，照常交给 xterm。
export function isCursorReset(params: (number | number[])[]) {
  const first = params[0];
  return !(Array.isArray(first) ? first[0] : first);
}

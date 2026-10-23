// 应用内提示/确认框（由 layout/DialogHost 渲染）。不用 window.alert/confirm：tauri-plugin-dialog 把它们
// 换成了原生 NSAlert（窄、按钮英文、中文长句被拦腰折行，与应用对话框风格不一），且换后的 confirm 返回
// Promise，`if (!confirm(..))` 永远拦不住；plugin-dialog 的 ask 同样是原生 NSAlert。多个请求排队，一个一个显示。

export type DialogRequest = {
  id: number;
  kind: 'confirm' | 'alert';
  title: string;
  message?: string;
  /** 单行补充信息（如文件名清单），超出省略，不在名字中间折行。 */
  detail?: string;
  confirmLabel: string;
  cancelLabel?: string;
  resolve: (ok: boolean) => void;
};

type DialogOptions = { title: string; message?: string; detail?: string; confirmLabel?: string };

const queue: DialogRequest[] = [];
const listeners = new Set<() => void>();
let nextId = 1;
// 第一个对话框弹出前的焦点（如终端的 textarea）。对话框没有 DialogTrigger，Radix 关闭时不会自己还焦点。
let focusBefore: { focus: (options?: FocusOptions) => void; isConnected?: boolean } | null = null;

function notify() {
  listeners.forEach(listener => listener());
}

export function subscribeDialogs(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function currentDialog(): DialogRequest | null {
  return queue[0] ?? null;
}

/** 关闭当前对话框：confirm 按 ok 返回，alert 总是结束。 */
export function settleDialog(ok: boolean) {
  const head = queue.shift();
  if (!head) return;
  head.resolve(ok);
  notify();
}

/** 最后一个对话框关闭后把焦点还给弹出前的元素，否则在终端里关掉报错后要再点一下才能继续输入。 */
export function restoreFocusAfterDialogs() {
  if (queue.length) return;
  const target = focusBefore;
  focusBefore = null;
  if (target && target.isConnected !== false) target.focus({ preventScroll: true });
}

function enqueue(request: Omit<DialogRequest, 'id' | 'resolve'>) {
  return new Promise<boolean>(resolve => {
    if (!queue.length && typeof document !== 'undefined') focusBefore = document.activeElement as typeof focusBefore;
    queue.push({ ...request, id: nextId++, resolve });
    notify();
  });
}

export function confirmDialog({ title, message, detail, confirmLabel = '确定', cancelLabel = '取消' }: DialogOptions & { cancelLabel?: string }) {
  return enqueue({ kind: 'confirm', title, message, detail, confirmLabel, cancelLabel });
}

export function alertDialog({ title, message, detail, confirmLabel = '知道了' }: DialogOptions) {
  return enqueue({ kind: 'alert', title, message, detail, confirmLabel }).then(() => undefined);
}

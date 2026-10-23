// 应用内提示/确认框（由 layout/DialogHost 渲染）。不用 window.alert/confirm：wry 没实现 WKWebView 的
// alert/confirm 回调，alert 静默不显示、confirm 恒为 false；也不用原生 NSAlert：窄、按钮是英文 Yes/No，
// 中文长句会被拦腰折行，和应用自己的对话框风格不一。多个请求排队，一个一个显示。

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

function enqueue(request: Omit<DialogRequest, 'id' | 'resolve'>) {
  return new Promise<boolean>(resolve => {
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

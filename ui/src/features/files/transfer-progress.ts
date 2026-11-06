import { formatFileSize } from './useFileList';

/** 后端 file:download-progress / file:upload-progress 事件的载荷；目录传输额外带文件计数。 */
export type TransferProgress = {
  remotePath?: string;
  transferred?: number;
  total?: number;
  filesDone?: number;
  filesTotal?: number;
};

export function describeProgress(verb: string, data: TransferProgress | null | undefined) {
  const total = data?.total || 0;
  const transferred = data?.transferred || 0;
  const progress = total > 0 ? Math.min(100, Math.round((transferred / total) * 100)) : 0;
  const name = String(data?.remotePath || '').split('/').filter(Boolean).pop() || '';
  const files = data?.filesTotal ? ` · ${data.filesDone || 0}/${data.filesTotal} 个文件` : '';
  return { progress, info: `正在${verb}: ${name} (${progress}% - ${formatFileSize(transferred)}/${formatFileSize(total)}${files})` };
}

export type TransferStatus = { active: boolean; progress: number; info: string };

// 进度条状态的小 store：传输时进度事件约每 100ms 一次，放进 React state 会让整个文件管理（两侧列表）
// 跟着 10Hz 重渲染；放这里只有订阅它的进度条重渲染。
export function createTransferStatusStore() {
  let status: TransferStatus = { active: false, progress: 0, info: '' };
  const listeners = new Set<() => void>();
  return {
    get: () => status,
    set: (next: TransferStatus | ((current: TransferStatus) => TransferStatus)) => {
      status = typeof next === 'function' ? next(status) : next;
      listeners.forEach(listener => listener());
    },
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

export type TransferStatusStore = ReturnType<typeof createTransferStatusStore>;

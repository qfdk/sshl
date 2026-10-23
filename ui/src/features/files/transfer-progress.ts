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

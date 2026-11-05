import { useCallback, useEffect, useRef, useState } from 'react';
import { alertDialog, confirmDialog, passwordDialog } from '../../lib/dialogs';
import { describeProgress, type TransferProgress } from './transfer-progress';
import { sudoErrorMessage, uploadWithSudoFallback } from './sudo-upload';

function parentDir(remotePath: string) {
  const i = remotePath.replace(/\/+$/, '').lastIndexOf('/');
  return i <= 0 ? '/' : remotePath.slice(0, i);
}

// 普通上传；目标目录无写权限时问一次是否用 sudo（见 sudo-upload.ts）。
function uploadWithSudo(sessionId: string, localPath: string, remotePath: string, isDirectory: boolean) {
  const file = (window as AppWindow).api.file;
  return uploadWithSudoFallback({
    normal: () => isDirectory ? file.uploadDirectory(sessionId, localPath, remotePath) : file.upload(sessionId, localPath, remotePath),
    sudo: password => file.uploadSudo(sessionId, localPath, remotePath, isDirectory, password),
    confirm: () => confirmDialog({
      title: '没有写入权限',
      message: '当前登录用户不能写入这个目录。要用 sudo 上传吗？文件会归目录的所有者。',
      detail: parentDir(remotePath),
      confirmLabel: '用 sudo 上传',
    }),
    askPassword: retry => passwordDialog({
      title: retry ? 'sudo 密码不正确' : '需要 sudo 密码',
      message: '输入当前登录用户的 sudo 密码。',
      confirmLabel: '上传',
    }),
  }).then(result => result?.error?.startsWith('SUDO_') ? { ...result, error: sudoErrorMessage(result.error) } : result);
}

type AppWindow = Window & { api?: any };

type TransferStatus = {
  active: boolean;
  progress: number;
  info: string;
};

export function useTransfer(sessionId: string | null, onUploadComplete: () => void, onDownloadComplete: () => void) {
  const [activeTransfers, setActiveTransfers] = useState(0);
  const activeRef = useRef(0);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [status, setStatus] = useState<TransferStatus>({ active: false, progress: 0, info: '' });

  useEffect(() => {
    const api = (window as AppWindow).api?.file;
    if (!api) return;
    const handleProgress = (verb: string, data: TransferProgress) => {
      setStatus({ active: true, ...describeProgress(verb, data) });
    };
    const stopDownload = api.onDownloadProgress?.((_event: unknown, data: any) => handleProgress('下载', data));
    const stopUpload = api.onUploadProgress?.((_event: unknown, data: any) => handleProgress('上传', data));
    return () => {
      stopDownload?.();
      stopUpload?.();
    };
  }, []);

  const run = useCallback(async (verb: string, name: string, operation: () => Promise<any>, onComplete: () => void) => {
    activeRef.current += 1;
    setActiveTransfers(activeRef.current);
    if (hideTimer.current) clearTimeout(hideTimer.current);
    setStatus({ active: true, progress: 0, info: `正在${verb}: ${name}` });
    try {
      const result = await operation();
      if (result?.success) {
        setStatus({ active: true, progress: 100, info: `${verb}完成` });
        onComplete();
      } else if (result?.cancelled) {
        // 用户在 sudo 确认或密码框里取消：不算失败，不弹报错
        setStatus({ active: true, progress: 0, info: `已取消${verb}` });
      } else {
        const message = result?.error || '未知错误';
        setStatus({ active: true, progress: 0, info: `${verb}失败` });
        void alertDialog({ title: `${verb}失败`, message });
      }
      hideTimer.current = setTimeout(() => setStatus(current => ({ ...current, active: false, progress: 0 })), 3000);
      return result;
    } catch (error: any) {
      setStatus({ active: true, progress: 0, info: `${verb}失败` });
      void alertDialog({ title: `${verb}失败`, message: String(error?.message || error) });
      setStatus(current => ({ ...current, active: false }));
      return null;
    } finally {
      activeRef.current = Math.max(0, activeRef.current - 1);
      setActiveTransfers(activeRef.current);
    }
  }, []);

  const uploadFile = useCallback((localPath: string, remotePath: string) => {
    if (!sessionId) {
      void alertDialog({ title: '未连接服务器', message: '请先连接到服务器。' });
      return Promise.resolve(null);
    }
    return run('上传', localPath.split(/[\\/]/).pop() || localPath, () => uploadWithSudo(sessionId, localPath, remotePath, false), onUploadComplete);
  }, [onUploadComplete, run, sessionId]);

  const downloadFile = useCallback((remotePath: string, localPath: string) => {
    if (!sessionId) {
      void alertDialog({ title: '未连接服务器', message: '请先连接到服务器。' });
      return Promise.resolve(null);
    }
    return run('下载', remotePath.split('/').pop() || remotePath, () => (window as AppWindow).api.file.download(sessionId, remotePath, localPath), onDownloadComplete);
  }, [onDownloadComplete, run, sessionId]);

  const uploadDirectory = useCallback((localPath: string, remotePath: string) => {
    if (!sessionId) {
      void alertDialog({ title: '未连接服务器', message: '请先连接到服务器。' });
      return Promise.resolve(null);
    }
    return run('上传文件夹', localPath.split(/[\\/]/).pop() || localPath, () => uploadWithSudo(sessionId, localPath, remotePath, true), onUploadComplete);
  }, [onUploadComplete, run, sessionId]);

  const downloadDirectory = useCallback((remotePath: string, localPath: string) => {
    if (!sessionId) {
      void alertDialog({ title: '未连接服务器', message: '请先连接到服务器。' });
      return Promise.resolve(null);
    }
    return run('下载文件夹', remotePath.split('/').pop() || remotePath, () => (window as AppWindow).api.file.downloadDirectory(sessionId, remotePath, localPath), onDownloadComplete);
  }, [onDownloadComplete, run, sessionId]);

  return { activeTransfers, status, uploadFile, downloadFile, uploadDirectory, downloadDirectory };
}

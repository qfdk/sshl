import { useCallback, useEffect, useRef, useState } from 'react';
import { formatFileSize } from './useFileList';

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
    const handleProgress = (verb: string, data: any) => {
      const total = data?.total || 0;
      const transferred = data?.transferred || 0;
      const progress = total > 0 ? Math.min(100, Math.round((transferred / total) * 100)) : 0;
      const name = String(data?.remotePath || '').split('/').pop() || '';
      setStatus({ active: true, progress, info: `正在${verb}: ${name} (${progress}% - ${formatFileSize(transferred)}/${formatFileSize(total)})` });
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
      } else {
        const message = result?.error || '未知错误';
        setStatus({ active: true, progress: 0, info: `${verb}失败` });
        window.alert(`${verb}失败: ${message}`);
      }
      hideTimer.current = setTimeout(() => setStatus(current => ({ ...current, active: false, progress: 0 })), 3000);
      return result;
    } catch (error: any) {
      setStatus({ active: true, progress: 0, info: `${verb}失败` });
      window.alert(`${verb}失败: ${error?.message || error}`);
      setStatus(current => ({ ...current, active: false }));
      return null;
    } finally {
      activeRef.current = Math.max(0, activeRef.current - 1);
      setActiveTransfers(activeRef.current);
    }
  }, []);

  const uploadFile = useCallback((localPath: string, remotePath: string) => {
    if (!sessionId) {
      window.alert('请先连接到服务器');
      return Promise.resolve(null);
    }
    return run('上传', localPath.split(/[\\/]/).pop() || localPath, () => (window as AppWindow).api.file.upload(sessionId, localPath, remotePath), onUploadComplete);
  }, [onUploadComplete, run, sessionId]);

  const downloadFile = useCallback((remotePath: string, localPath: string) => {
    if (!sessionId) {
      window.alert('请先连接到服务器');
      return Promise.resolve(null);
    }
    return run('下载', remotePath.split('/').pop() || remotePath, () => (window as AppWindow).api.file.download(sessionId, remotePath, localPath), onDownloadComplete);
  }, [onDownloadComplete, run, sessionId]);

  const uploadDirectory = useCallback((localPath: string, remotePath: string) => {
    if (!sessionId) {
      window.alert('请先连接到服务器');
      return Promise.resolve(null);
    }
    return run('上传文件夹', localPath.split(/[\\/]/).pop() || localPath, () => (window as AppWindow).api.file.uploadDirectory(sessionId, localPath, remotePath), onUploadComplete);
  }, [onUploadComplete, run, sessionId]);

  const downloadDirectory = useCallback((remotePath: string, localPath: string) => {
    if (!sessionId) {
      window.alert('请先连接到服务器');
      return Promise.resolve(null);
    }
    return run('下载文件夹', remotePath.split('/').pop() || remotePath, () => (window as AppWindow).api.file.downloadDirectory(sessionId, remotePath, localPath), onDownloadComplete);
  }, [onDownloadComplete, run, sessionId]);

  return { activeTransfers, status, uploadFile, downloadFile, uploadDirectory, downloadDirectory };
}

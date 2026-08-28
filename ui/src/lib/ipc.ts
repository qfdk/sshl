import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ask, open } from '@tauri-apps/plugin-dialog';

type EventCallback = (event: unknown, payload: any) => void;
type CallShape = 'raw' | string | undefined;

function errorMessage(error: unknown) {
  return typeof error === 'string' ? error : (error as { message?: string })?.message || JSON.stringify(error);
}

function call<T = any>(command: string, args?: Record<string, unknown>, shape?: CallShape): Promise<T> {
  return invoke(command, args).then(
    payload => {
      if (shape === 'raw') return payload as T;
      if (payload && typeof payload === 'object' && !Array.isArray(payload)) return { success: true, ...(payload as object) } as T;
      if (typeof shape === 'string') return { success: true, [shape]: payload } as T;
      return { success: true, value: payload } as T;
    },
    error => {
      const message = errorMessage(error);
      console.error(`[ipc] ${command} failed:`, message);
      return { success: false, error: message } as T;
    },
  );
}

function onEvent(name: string) {
  return (callback: EventCallback) => {
    let unlisten: (() => void) | null = null;
    void listen(name, event => {
      try { callback({}, event.payload); } catch (error) { console.error(`[ipc] ${name} callback:`, error); }
    }).then(stop => { unlisten = stop; }).catch(error => console.error(`[ipc] listen ${name}:`, error));
    return () => { try { unlisten?.(); } catch {} };
  };
}

export const api = {
  ssh: {
    connect: (details: unknown, attemptId?: string) => call('ssh_connect', { details, attemptId }),
    cancelConnect: (attemptId: string) => call('ssh_cancel_connect', { attemptId }),
    validateSession: (sessionId: string) => call('ssh_validate_session', { sessionId }, 'alive'),
    disconnect: (sessionId: string) => call('ssh_disconnect', { sessionId }),
    execute: (sessionId: string, command: string) => call('ssh_execute', { sessionId, command }, 'output'),
    sendData: (sessionId: string, data: string) => call('ssh_send_data', { sessionId, data }),
    fillPassword: (sessionId: string, kind?: string) => call('ssh_fill_password', { sessionId, kind }),
    resize: (sessionId: string, cols: number, rows: number) => call('ssh_resize', { sessionId, cols, rows }),
    refreshPrompt: (sessionId: string) => call('ssh_refresh_prompt', { sessionId }),
    // 激活后快照由后端作为首个 ssh:data 事件发出，与后续数据同通道保序。
    activateSession: (sessionId: string) => call('ssh_activate_session', { sessionId }),
    getSessionBuffer: (sessionId: string) => call('ssh_get_session_buffer', { sessionId }, 'buffer'),
    onData: onEvent('ssh:data'),
    onClosed: (callback: EventCallback) => onEvent('ssh:closed')((event, payload) => {
      console.warn('[ssh:closed]', payload?.sessionId, 'reason:', payload?.reason, 'code:', payload?.code);
      callback(event, payload);
    }),
  },
  file: {
    list: (sessionId: string, path: string) => call('file_list', { sessionId, path }, 'files'),
    listLocal: (directory: string) => call('file_list_local', { directory }, 'files'),
    getHomeDir: () => invoke<string>('file_get_home_dir'),
    listSystemFonts: () => invoke<string[]>('list_system_fonts'),
    upload: (sessionId: string, localPath: string, remotePath: string) => call('file_upload', { sessionId, localPath, remotePath }),
    download: (sessionId: string, remotePath: string, localPath: string) => call('file_download', { sessionId, remotePath, localPath }),
    deleteLocal: (filePath: string) => call('file_delete_local', { filePath }),
    deleteLocalDirectory: (dirPath: string) => call('file_delete_local_directory', { dirPath }),
    createRemoteDirectory: (sessionId: string, remotePath: string) => call('file_create_remote_directory', { sessionId, remotePath }),
    uploadDirectory: (sessionId: string, localPath: string, remotePath: string) => call('file_upload_directory', { sessionId, localPath, remotePath }),
    downloadDirectory: (sessionId: string, remotePath: string, localPath: string) => call('file_download_directory', { sessionId, remotePath, localPath }),
    changePermissions: (sessionId: string, remotePath: string, permissions: string | number) => call('file_change_permissions', { sessionId, remotePath, permissions: typeof permissions === 'string' ? parseInt(permissions, 8) : permissions }),
    changeOwner: (sessionId: string, remotePath: string, owner: string, group: string) => call('file_change_owner', { sessionId, remotePath, owner, group }),
    onDownloadProgress: onEvent('file:download-progress'),
    onUploadProgress: onEvent('file:upload-progress'),
  },
  config: {
    getConnections: () => invoke<any[]>('config_get_connections'),
    getConnectionGroups: () => invoke<string[]>('config_get_connection_groups'),
    saveConnection: (connection: unknown) => call('config_save_connection', { connection }),
    deleteConnection: (id: string) => call('config_delete_connection', { id }),
    applyConnectionLayout: (layout: unknown, groups?: string[]) => call('config_apply_connection_layout', { layout, groups: groups ?? null }),
    onConnectionsUpdated: (callback: () => void) => {
      let unlisten: (() => void) | null = null;
      void listen('connections:updated', () => { try { callback(); } catch (error) { console.error(error); } })
        .then(stop => { unlisten = stop; }).catch(error => console.error('[ipc] listen connections:updated:', error));
      return () => { try { unlisten?.(); } catch {} };
    },
  },
  cred: {
    list: (connectionId: string) => invoke<any>('cred_list', { connectionId }),
    set: (connectionId: string, account: string, password: string) => call('cred_set', { connectionId, account, password }),
    delete: (connectionId: string, account: string) => call('cred_delete', { connectionId, account }),
  },
  dialog: {
    selectFile: async () => {
      const result = await open({ multiple: false, directory: false });
      if (result == null) return { canceled: true, filePaths: [] };
      return { canceled: false, filePaths: [Array.isArray(result) ? result[0] : result] };
    },
    selectDirectory: async () => {
      const result = await open({ multiple: false, directory: true });
      if (result == null) return { canceled: true, filePaths: [] };
      return { canceled: false, filePaths: [Array.isArray(result) ? result[0] : result] };
    },
    confirm: (message: string, title?: string) => ask(message, { title: title || '确认', kind: 'warning' }),
  },
};

const browserWindow = window as Window & { api?: typeof api };
browserWindow.api = api;
export type Api = typeof api;

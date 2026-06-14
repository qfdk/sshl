// IPC bridge — unifies Electron preload (window.api) and Tauri 2 (window.__TAURI__).
//
// Electron main.js wraps every reply as { success: true, ...payload } and every error
// as { success: false, error: msg }. The frontend depends on that shape. This bridge
// replicates that contract on top of Tauri's raw invoke (which throws on Err and returns
// raw payloads on Ok).

(function () {
  'use strict';

  if (window.api && window.api.ssh && typeof window.api.ssh.connect === 'function') {
    return; // Electron preload already wired
  }

  const tauri = window.__TAURI__;
  if (!tauri) {
    console.error('[ipc-bridge] neither window.api nor window.__TAURI__ available');
    return;
  }

  const rawInvoke = tauri.core.invoke;
  const listen = tauri.event.listen;

  // Wrap an invoke so success becomes { success:true, ...payload } and rejection
  // becomes { success:false, error } AND throws an Error (covers both call styles).
  const call = (cmd, args, shape) => {
    return rawInvoke(cmd, args).then(
      (payload) => {
        if (shape === 'raw') return payload;
        if (payload && typeof payload === 'object' && !Array.isArray(payload)) {
          return { success: true, ...payload };
        }
        // primitive / array — wrap under a named key if shape provided
        if (typeof shape === 'string') return { success: true, [shape]: payload };
        return { success: true, value: payload };
      },
      (err) => {
        const msg = typeof err === 'string' ? err : (err && err.message) || JSON.stringify(err);
        console.error(`[ipc-bridge] ${cmd} failed:`, msg);
        return { success: false, error: msg };
      }
    );
  };

  const onEvent = (name) => (callback) => {
    let unlistenFn = null;
    const stop = () => { try { unlistenFn && unlistenFn(); } catch {} };
    listen(name, (e) => {
      try { callback({}, e.payload); } catch (err) { console.error(`[ipc-bridge] ${name} cb:`, err); }
    }).then((un) => { unlistenFn = un; }).catch((err) => console.error(`[ipc-bridge] listen ${name}:`, err));
    return stop;
  };

  window.api = {
    ssh: {
      connect: (details) => call('ssh_connect', { details }),                     // {success, sessionId}
      disconnect: (sessionId) => call('ssh_disconnect', { sessionId }),           // {success}
      execute: (sessionId, command) => call('ssh_execute', { sessionId, command }, 'output'),
      sendData: (sessionId, data) => call('ssh_send_data', { sessionId, data }),
      resize: (sessionId, cols, rows) => call('ssh_resize', { sessionId, cols, rows }),
      refreshPrompt: (sessionId) => call('ssh_refresh_prompt', { sessionId }),
      activateSession: (sessionId) => call('ssh_activate_session', { sessionId }),
      getSessionBuffer: (sessionId) => call('ssh_get_session_buffer', { sessionId }, 'buffer'),
      onData: onEvent('ssh:data'),
      onClosed: (callback) => onEvent('ssh:closed')((evt, payload) => {
        console.warn('[ssh:closed]', payload?.sessionId, 'reason:', payload?.reason, 'code:', payload?.code);
        callback(evt, payload);
      }),
    },

    file: {
      list: (sessionId, path) => call('file_list', { sessionId, path }, 'files'),
      listLocal: (directory) => call('file_list_local', { directory }, 'files'),
      // getHomeDir is consumed as a raw string by ui-manager and file-manager
      getHomeDir: () => call('file_get_home_dir', undefined, 'raw'),
      listSystemFonts: () => call('list_system_fonts', undefined, 'raw'),
      upload: (sessionId, localPath, remotePath) =>
        call('file_upload', { sessionId, localPath, remotePath }),
      download: (sessionId, remotePath, localPath) =>
        call('file_download', { sessionId, remotePath, localPath }),
      deleteLocal: (filePath) => call('file_delete_local', { filePath }),
      deleteLocalDirectory: (dirPath) => call('file_delete_local_directory', { dirPath }),
      createRemoteDirectory: (sessionId, remotePath) =>
        call('file_create_remote_directory', { sessionId, remotePath }),
      uploadDirectory: (sessionId, localPath, remotePath) =>
        call('file_upload_directory', { sessionId, localPath, remotePath }),
      downloadDirectory: (sessionId, remotePath, localPath) =>
        call('file_download_directory', { sessionId, remotePath, localPath }),
      changePermissions: (sessionId, remotePath, permissions) =>
        call('file_change_permissions', {
          sessionId,
          remotePath,
          permissions: typeof permissions === 'string' ? parseInt(permissions, 8) : permissions
        }),
      changeOwner: (sessionId, remotePath, owner, group) =>
        call('file_change_owner', { sessionId, remotePath, owner, group }),
      onDownloadProgress: onEvent('file:download-progress'),
      onUploadProgress: onEvent('file:upload-progress'),
    },

    config: {
      // Frontend code iterates over the array directly — return raw, no wrapping.
      getConnections: () => call('config_get_connections', undefined, 'raw'),
      saveConnection: (connection) => call('config_save_connection', { connection }),
      deleteConnection: (id) => call('config_delete_connection', { id }),
      onConnectionsUpdated: (callback) => {
        let unlistenFn = null;
        const stop = () => { try { unlistenFn && unlistenFn(); } catch {} };
        listen('connections:updated', () => { try { callback(); } catch (err) { console.error(err); } })
          .then((un) => { unlistenFn = un; })
          .catch((err) => console.error('[ipc-bridge] listen connections:updated:', err));
        return stop;
      },
    },

    dialog: {
      selectFile: async () => {
        const { open } = tauri.dialog || (await import('@tauri-apps/plugin-dialog'));
        const result = await open({ multiple: false, directory: false });
        if (result == null) return { canceled: true, filePaths: [] };
        return { canceled: false, filePaths: [Array.isArray(result) ? result[0] : result] };
      },
      selectDirectory: async () => {
        const { open } = tauri.dialog || (await import('@tauri-apps/plugin-dialog'));
        const result = await open({ multiple: false, directory: true });
        if (result == null) return { canceled: true, filePaths: [] };
        return { canceled: false, filePaths: [Array.isArray(result) ? result[0] : result] };
      },
      confirm: async (message, title) => {
        const { ask } = tauri.dialog || (await import('@tauri-apps/plugin-dialog'));
        return await ask(message, { title: title || '确认', kind: 'warning' });
      },
    },
  };
})();

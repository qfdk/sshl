// main-entry.js
// 应用程序入口文件，导入所有模块并初始化应用

// 导入所有管理器模块
import sessionManager from './session-manager.js';
import terminalManager from './terminal-manager.js';
import fileManager from './file-manager.js';
import connectionManager from './connection-manager.js';
import uiManager from './ui-manager.js';
import { initSettingsUI } from './settings.js';

// 添加自定义样式
function addCustomStyles() {
    // 终端相关CSS
    const terminalCSS = `
    .terminal-view {
        position: relative;
        display: flex;
        flex-direction: column;
        height: 100%;
        background-color: #1e1e1e;
        color: #f0f0f0;
        overflow: hidden;
    }

    .terminal-content {
        position: relative;
        flex: 1;
        overflow: hidden;
        min-height: 0; /* Critical for proper flex sizing */
        display: flex; /* Ensure it fills space */
    }

    .terminal-container {
        width: 100%;
        height: 100%;
        background-color: #1e1e1e !important;
        flex: 1; /* Fill available space */
        display: flex;
        flex-direction: column;
    }

    .terminal-container .xterm {
        height: 100%;
        flex: 1;
    }

    /* Fix terminal layers */
    .terminal-container .xterm-screen,
    .terminal-container .xterm-viewport {
        width: 100% !important;
        height: 100% !important;
    }

    /* Remove absolute positioning causing overlays */
    .terminal-placeholder {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
        background-color: #1e1e1e;
        z-index: 10;
    }

    /* Hide overlays */
    .terminal-content > div:not(#terminal-container):not(#terminal-placeholder) {
        display: none !important;
    }

    /* Terminal tab style fixes */
    .terminal-tabs {
        background-color: #252526;
        padding: 4px 4px 0;
        border-bottom: 1px solid #333;
    }

    .terminal-tab {
        background-color: #2d2d2d;
        color: #ccc;
        border-radius: 4px 4px 0 0;
        padding: 6px 12px;
        font-size: 13px;
    }

    .terminal-tab.active {
        background-color: #1e1e1e;
        color: #fff;
    }
    `;

    // 右键菜单样式
    const menuCSS = `
    #context-menu {
        position: fixed;
        background-color: white;
        border: 1px solid #ddd;
        border-radius: 4px;
        padding: 5px 0;
        box-shadow: 0 2px 5px rgba(0,0,0,0.2);
        z-index: 1000;
    }

    #context-menu div {
        padding: 8px 12px;
        cursor: pointer;
        color: #333;
        display: flex;
        align-items: center;
        gap: 8px;
    }

    #context-menu div:hover {
        background-color: #f3f4f6;
    }

    #context-menu div.download::before {
        content: "";
        display: inline-block;
        width: 16px;
        height: 16px;
        background-image: url("${window.Icons.dataUri('download', '#333333')}");
        background-repeat: no-repeat;
        background-position: center;
    }

    #context-menu div.upload::before {
        content: "";
        display: inline-block;
        width: 16px;
        height: 16px;
        background-image: url("${window.Icons.dataUri('upload', '#333333')}");
        background-repeat: no-repeat;
        background-position: center;
    }

    #context-menu div.delete::before {
        content: "";
        display: inline-block;
        width: 16px;
        height: 16px;
        background-image: url("${window.Icons.dataUri('trash-2', '#333333')}");
        background-repeat: no-repeat;
        background-position: center;
    }
    #context-menu div.create-directory::before {
        content: "";
        display: inline-block;
        width: 16px;
        height: 16px;
        background-image: url("${window.Icons.dataUri('folder-plus', '#374151')}");
        background-repeat: no-repeat;
        background-position: center;
    }
    `;

    // 权限对话框样式
    const permissionsCSS = `
    #permissions-dialog {
        z-index: 10000;
    }

    #permissions-dialog .dialog-overlay {
        position: fixed;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        background: rgba(0, 0, 0, 0.6);
        backdrop-filter: blur(4px);
    }

    .permissions-dialog-content {
        position: fixed;
        top: 50%;
        left: 50%;
        transform: translate(-50%, -50%);
        background: white;
        border-radius: 16px;
        box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.25);
        max-width: 700px;
        width: 90%;
        max-height: 90vh;
        overflow-y: auto;
        padding: 0;
        animation: dialogSlideIn 0.3s ease-out;
    }

    @keyframes dialogSlideIn {
        from { 
            opacity: 0; 
            transform: translate(-50%, -45%) scale(0.95);
        }
        to { 
            opacity: 1; 
            transform: translate(-50%, -50%) scale(1);
        }
    }

    .dialog-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 24px 28px 20px;
        border-bottom: 1px solid #e5e7eb;
        background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        color: white;
        border-radius: 16px 16px 0 0;
    }

    .dialog-header h3 {
        margin: 0;
        font-size: 18px;
        font-weight: 600;
        display: flex;
        align-items: center;
        gap: 10px;
    }

    .close-btn {
        background: rgba(255, 255, 255, 0.2);
        border: none;
        border-radius: 8px;
        width: 32px;
        height: 32px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background-color 0.2s;
        color: white;
    }

    .close-btn:hover {
        background: rgba(255, 255, 255, 0.3);
    }

    .file-info {
        padding: 20px 28px;
        background: #f8fafc;
        border-bottom: 1px solid #e5e7eb;
    }

    .file-path {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-bottom: 12px;
        font-size: 14px;
        color: #374151;
    }

    .file-path span {
        font-family: monospace;
        background: white;
        padding: 4px 8px;
        border-radius: 6px;
        border: 1px solid #d1d5db;
        word-break: break-all;
    }

    .current-perms {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 14px;
    }

    .current-perms .label {
        color: #6b7280;
        font-weight: 500;
    }

    .perm-code {
        background: #1f2937;
        color: #10b981;
        padding: 4px 8px;
        border-radius: 6px;
        font-family: monospace;
        font-weight: 600;
        font-size: 13px;
    }
    .octal-input-section {
        margin-bottom: 16px;
    }
    .octal-input-section label {
        display: block;
        margin-bottom: 8px;
        font-weight: 600;
        color: #374151;
        font-size: 14px;
    }

    .input-with-preview {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-bottom: 16px;
    }

    #new-permissions {
        font-family: 'SF Mono', Monaco, 'Cascadia Code', 'Roboto Mono', Consolas, 'Courier New', monospace;
        font-size: 20px;
        text-align: center;
        font-weight: 700;
        border: 2px solid #d1d5db;
        border-radius: 10px;
        width: 80px;
        background: white;
        transition: all 0.2s;
    }

    #new-permissions:focus {
        outline: none;
        border-color: #667eea;
        box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.1);
    }

    .preview-text {
        font-family: monospace;
        font-size: 16px;
        font-weight: 600;
        background: #f3f4f6;
        border-radius: 8px;
        min-width: 120px;
        text-align: center;
        transition: all 0.2s;
    }

    .preview-text.valid {
        background: #dcfce7;
        color: #166534;
        border: 1px solid #bbf7d0;
    }

    .preview-text.invalid {
        background: #fef2f2;
        color: #dc2626;
        border: 1px solid #fecaca;
    }

    .common-permissions {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
    }

    .common-permissions .label {
        font-size: 13px;
        color: #6b7280;
        font-weight: 500;
        margin-right: 4px;
    }

    .perm-preset {
        background: #f3f4f6;
        border: 1px solid #d1d5db;
        border-radius: 6px;
        padding: 6px 12px;
        font-family: monospace;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.2s;
        color: #374151;
    }

    .perm-preset:hover {
        background: #667eea;
        color: white;
        border-color: #667eea;
        transform: translateY(-1px);
    }

    .permissions-visual {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: 20px;
        padding-top:16px;
    }

    .permission-group {
        background: white;
        border: 1px solid #e5e7eb;
        border-radius: 12px;
        padding: 20px;
        transition: all 0.2s;
    }

    .permission-group:hover {
        box-shadow: 0 4px 12px rgba(0, 0, 0, 0.05);
        border-color: #d1d5db;
    }

    .group-header {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-bottom: 16px;
        font-weight: 600;
        color: #374151;
        font-size: 14px;
        padding-bottom: 8px;
        border-bottom: 2px solid #f3f4f6;
    }

    .permission-checkboxes {
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .checkbox-item {
        display: flex;
        align-items: center;
        gap: 12px;
        cursor: pointer;
        padding: 8px 12px;
        border-radius: 8px;
        transition: all 0.2s;
        position: relative;
    }

    .checkbox-item:hover {
        background: #f8fafc;
    }

    .checkbox-item.read:hover { background: #fef2f2; }
    .checkbox-item.write:hover { background: #fffbeb; }
    .checkbox-item.exec:hover { background: #f0fdf4; }

    .checkbox-item input[type="checkbox"] {
        display: none;
    }

    .checkmark {
        width: 20px;
        height: 20px;
        border: 2px solid #d1d5db;
        border-radius: 6px;
        position: relative;
        transition: all 0.2s;
        flex-shrink: 0;
    }

    .checkbox-item input[type="checkbox"]:checked + .checkmark {
        background: #667eea;
        border-color: #667eea;
    }

    .checkbox-item.read input[type="checkbox"]:checked + .checkmark {
        background: #ef4444;
        border-color: #ef4444;
    }

    .checkbox-item.write input[type="checkbox"]:checked + .checkmark {
        background: #f59e0b;
        border-color: #f59e0b;
    }

    .checkbox-item.exec input[type="checkbox"]:checked + .checkmark {
        background: #10b981;
        border-color: #10b981;
    }

    .checkmark::after {
        content: "";
        position: absolute;
        display: none;
        left: 6px;
        top: 2px;
        width: 6px;
        height: 10px;
        border: solid white;
        border-width: 0 2px 2px 0;
        transform: rotate(45deg);
    }

    .checkbox-item input[type="checkbox"]:checked + .checkmark::after {
        display: block;
    }

    .perm-label {
        font-size: 14px;
        font-weight: 500;
        color: #374151;
        flex: 1;
    }

    .checkbox-item code {
        background: #f3f4f6;
        padding: 2px 6px;
        border-radius: 4px;
        font-family: monospace;
        font-size: 12px;
        font-weight: 600;
        color: #6b7280;
    }

    .dialog-actions {
        padding: 20px 28px 28px;
        background: #f8fafc;
        border-top: 1px solid #e5e7eb;
        display: flex;
        justify-content: flex-end;
        gap: 12px;
        border-radius: 0 0 16px 16px;
    }

    .btn {
        padding: 10px 20px;
        border-radius: 8px;
        font-weight: 600;
        font-size: 14px;
        cursor: pointer;
        transition: all 0.2s;
        display: flex;
        align-items: center;
        gap: 6px;
        border: none;
    }

    .btn-secondary {
        background: white;
        color: #374151;
        border: 1px solid #d1d5db;
    }

    .btn-secondary:hover {
        background: #f9fafb;
        border-color: #9ca3af;
    }

    .btn-primary {
        background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        color: white;
        border: 1px solid transparent;
    }

    .btn-primary:hover {
        transform: translateY(-1px);
        box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
    }

    .permissions-cell {
        font-family: monospace;
        padding: 4px 8px;
        border-radius: 4px;
        transition: all 0.2s;
    }

    .permissions-cell:hover {
        transform: translateY(-1px);
    }

    .owner-cell {
        color: #6b7280;
        font-weight: 600;
        background: #f8fafc;
        padding: 4px 8px;
        border-radius: 4px;
    }
    `;

    // 额外CSS修复
    const extraCSS = `
    /* Fix xterm sizing */
    .xterm {
        padding: 0;
        margin: 0;
    }

    /* Remove scrollbar padding */
    .xterm-viewport::-webkit-scrollbar {
        width: 10px;
        height: 10px;
    }

    .xterm-viewport::-webkit-scrollbar-track {
        background: #1e1e1e;
    }

    .xterm-viewport::-webkit-scrollbar-thumb {
        background: #555;
        border-radius: 5px;
    }

    /* Fix terminal fullscreen issue */
    #terminal-tab {
        display: flex;
        flex-direction: column;
        height: 100%;
        padding: 0;
        margin: 0;
        overflow: hidden;
    }

    /* Override any potential padding from parent elements */
    .tab-pane#terminal-tab {
        padding: 0 !important;
        margin: 0 !important;
    }

    /* Ensure the terminal background color matches */
    .tab-pane#terminal-tab, .terminal-view, .terminal-content, .terminal-container {
        background-color: #1e1e1e;
    }

    /* Make text white in file manager */
    .file-manager {
        color: #333;
    }

    /* Ensure the dropdown menu and search contexts are readable */
    .search-box input {
        color: #333;
    }
    `;

    const tooltipCSS = `
    /* Tooltip styles */
    .custom-tooltip {
        position: fixed;
        background-color: #333;
        color: white;
        padding: 6px 10px;
        border-radius: 4px;
        font-size: 12px;
        z-index: 1000;
        opacity: 0;
        transition: opacity 0.2s ease;
        pointer-events: none;
        white-space: nowrap;
        box-shadow: 0 2px 5px rgba(0,0,0,0.2);
        max-width: 200px;
        overflow: visible; /* Changed to allow arrow to be visible outside the box */
        margin: 0;
    }

    .custom-tooltip.visible {
        opacity: 1;
    }

    /* Dedicated arrow element */
    .tooltip-arrow {
        position: absolute;
        left: -5px;
        top: 50%;
        width: 0;
        height: 0;
        transform: translateY(-50%);
        border-style: solid;
        border-width: 5px 5px 5px 0;
        border-color: transparent #333 transparent transparent;
    }
    `;

    // 添加所有样式
    const customStyle = document.createElement('style');
    customStyle.textContent = `
      /* 初始化时隐藏终端选项卡内容 */
      #terminal-tab:not(.active) {
        display: none;
      }
      /* 确保终端容器和终端背景颜色一致 */
      .terminal-container, .terminal-container .terminal {
        background-color: #1e1e1e !important;
      }
      
      ${terminalCSS}
      ${menuCSS}
      ${permissionsCSS}
      ${extraCSS}
      ${tooltipCSS}
    `;
    document.head.appendChild(customStyle);
}

// 初始化应用程序
function initializeApp() {
    console.log('应用初始化开始');

    // 添加自定义样式
    addCustomStyles();

    // 设置全局变量和引用，使模块能够相互访问
    window.sessionManager = sessionManager;
    window.terminalManager = terminalManager;
    window.fileManager = fileManager;
    window.connectionManager = connectionManager;
    window.uiManager = uiManager;
    window.activeTabId = 'terminal';  // 默认活动标签
    window.currentSessionId = null;   // 当前会话ID

    // 初始化UI事件监听
    uiManager.initUIEvents();

    // 初始化设置面板
    initSettingsUI();

    // 设置路径输入框的回车键处理
    uiManager.setupEnterKeyHandler('remote-path', path => fileManager.loadRemoteFiles(path));
    uiManager.setupEnterKeyHandler('local-path', path => fileManager.loadLocalFiles(path));

    // 设置文件传输监听
    fileManager.setupFileTransferListeners();

    // 设置SSH数据处理和连接关闭处理
    connectionManager.setupSSHHandlers();

    // 加载连接列表
    connectionManager.loadConnections();

    // 设置连接更新监听
    if (window.api && window.api.config && window.api.config.onConnectionsUpdated) {
        window.api.config.onConnectionsUpdated(() => {
            connectionManager.loadConnections();
        });
    }

    // 下载进度监听
    if (window.api && window.api.file) {
        window.api.file.onDownloadProgress((event, progressData) => {
            // 更新进度条
            const progressBar = document.getElementById('transfer-progress-bar');
            const transferInfo = document.getElementById('transfer-info');

            if (progressBar && transferInfo) {
                // 显示传输状态
                uiManager.showTransferStatus(true);

                // 更新进度条宽度
                progressBar.style.width = `${progressData.progress}%`;

                // 更新信息文本
                const fileName = fileManager.path.basename(progressData.remotePath);
                const downloadedSize = fileManager.formatFileSize(progressData.downloadedBytes || progressData.completedSize);
                const totalSize = fileManager.formatFileSize(progressData.fileSize || progressData.totalSize);

                transferInfo.textContent = `正在下载: ${fileName} (${progressData.progress}% - ${downloadedSize}/${totalSize})`;

                // 完成后隐藏状态（带延迟）
                if (progressData.progress >= 100) {
                    transferInfo.textContent = '下载完成';
                    setTimeout(() => {
                        progressBar.style.width = '0%';
                        uiManager.showTransferStatus(false);
                    }, 3000);
                }
            }
        });
    }

    // 添加连接项点击事件委托
    document.addEventListener('click', async function (event) {
        // 断开连接按钮（必须放在最前，避免冒泡触发其它处理）
        if (event.target.closest('.disconnect-connection')) {
            const btn = event.target.closest('.disconnect-connection');
            const sessionId = btn.getAttribute('data-session-id');
            event.stopPropagation();
            if (sessionId && window.terminalManager) {
                await window.terminalManager.disconnectSession(sessionId);
            }
            return;
        }

        // 编辑连接按钮
        if (event.target.closest('.edit-connection')) {
            const btn = event.target.closest('.edit-connection');
            const id = btn.getAttribute('data-id');

            try {
                if (window.api && window.api.config) {
                    const connections = await window.api.config.getConnections();
                    const connection = connections.find(c => c.id === id);

                    if (connection) {
                        // 填充编辑表单并显示对话框
                        connectionManager.showEditConnectionDialog(connection);
                    }
                }
            } catch (error) {
                console.error('编辑连接失败:', error);
            }

            event.stopPropagation();  // 阻止事件冒泡，不触发连接项的事件
            return;
        }

        // 删除连接按钮 (必须放在连接项处理前)
        if (event.target.closest('.delete-connection')) {
            const btn = event.target.closest('.delete-connection');
            const id = btn.getAttribute('data-id');

            try {
                if (await window.api.dialog.confirm('确定要删除这个连接吗?', '删除连接')) {
                    if (window.api && window.api.config) {
                        const result = await window.api.config.deleteConnection(id);
                        if (result) {
                            await connectionManager.loadConnections();
                        }
                    }
                }
            } catch (error) {
                console.error('删除连接失败:', error);
            }

            event.stopPropagation();  // 阻止事件冒泡，不触发连接项的事件
            return;
        }
    });

    // 初始化终端占位符
    const placeholder = document.getElementById('terminal-placeholder');
    if (placeholder) {
        placeholder.classList.remove('hidden');
    }

    console.log('应用初始化完成');
}

// 检测es模块兼容性
function isEsModulesSupported() {
    try {
        new Function('import("")');
        return true;
    } catch (err) {
        return false;
    }
}

// 如果不支持ES模块，显示一个错误信息
if (!isEsModulesSupported()) {
    document.body.innerHTML = `
        <div style="padding: 20px; text-align: center; font-family: sans-serif;">
            <h2>浏览器不支持</h2>
            <p>您的浏览器不支持现代JavaScript模块系统，请升级到最新版本的浏览器。</p>
        </div>
    `;
} else {
    // 当文档加载完成时执行初始化
    document.addEventListener('DOMContentLoaded', initializeApp);
}

export {
    sessionManager,
    terminalManager,
    fileManager,
    connectionManager,
    uiManager
};

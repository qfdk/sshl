// main-entry.js
// 应用程序入口文件，导入所有模块并初始化应用

// 导入所有管理器模块
import sessionManager from './session-manager.js';
import terminalManager from './terminal-manager.js';
import fileManager from './file-manager.js';
import connectionManager from './connection-manager.js';
import uiManager from './ui-manager.js';
import { initSettingsUI } from './settings.js';

// 添加自定义样式（仅注入依赖 Icons.dataUri 的右键菜单图标样式；其余已静态化到 app-runtime.css）
function addCustomStyles() {
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

    // 仅注入依赖 Icons.dataUri 的右键菜单图标样式；其余已静态化到 app-runtime.css
    const customStyle = document.createElement('style');
    customStyle.textContent = menuCSS;
    document.head.appendChild(customStyle);
}

// 初始化应用程序
function initializeApp() {
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

    // 传输进度监听（下载 + 上传）
    if (window.api && window.api.file) {
        // 后端进度事件统一为 { remotePath, transferred, total }，前端据此算百分比。
        const renderTransfer = (verb, d) => {
            const progressBar = document.getElementById('transfer-progress-bar');
            const transferInfo = document.getElementById('transfer-info');
            if (!progressBar || !transferInfo) return;

            uiManager.showTransferStatus(true);
            const total = d.total || 0;
            const pct = total > 0 ? Math.min(100, Math.round((d.transferred / total) * 100)) : 0;
            progressBar.style.width = `${pct}%`;

            const fileName = fileManager.path.basename(d.remotePath);
            const done = fileManager.formatFileSize(d.transferred || 0);
            const all = fileManager.formatFileSize(total);
            transferInfo.textContent = `正在${verb}: ${fileName} (${pct}% - ${done}/${all})`;

            if (pct >= 100) {
                transferInfo.textContent = `${verb}完成`;
                setTimeout(() => {
                    progressBar.style.width = '0%';
                    uiManager.showTransferStatus(false);
                }, 3000);
            }
        };

        window.api.file.onDownloadProgress((_event, d) => renderTransfer('下载', d));
        window.api.file.onUploadProgress((_event, d) => renderTransfer('上传', d));
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

    setupPaneDivider();
}

function setupPaneDivider() {
    const divider = document.getElementById('pane-divider');
    if (!divider) return;
    const split = divider.parentElement;
    const localPane = split.querySelector('.local-pane');
    if (!split || !localPane) return;

    const STORAGE_KEY = 'sshl.localPaneFlex';
    const saved = parseFloat(localStorage.getItem(STORAGE_KEY));
    if (saved && saved > 0) {
        localPane.style.flexGrow = String(saved);
    }

    let startX = 0;
    let startLocalW = 0;
    let startRemoteW = 0;
    let totalFlex = 0;

    const onMove = (e) => {
        const dx = e.clientX - startX;
        const newLocalW = Math.max(220, Math.min(startLocalW + dx, startLocalW + startRemoteW - 220));
        const ratio = newLocalW / (startLocalW + startRemoteW);
        const localFlex = ratio * totalFlex;
        const remoteFlex = totalFlex - localFlex;
        localPane.style.flexGrow = String(localFlex);
        split.querySelector('.remote-pane').style.flexGrow = String(remoteFlex);
    };

    const onUp = () => {
        divider.classList.remove('dragging');
        document.body.style.cursor = '';
        document.removeEventListener('mousemove', onMove);
        document.removeEventListener('mouseup', onUp);
        localStorage.setItem(STORAGE_KEY, localPane.style.flexGrow || '1');
    };

    divider.addEventListener('mousedown', (e) => {
        e.preventDefault();
        const localRect = localPane.getBoundingClientRect();
        const remotePane = split.querySelector('.remote-pane');
        const remoteRect = remotePane.getBoundingClientRect();
        startX = e.clientX;
        startLocalW = localRect.width;
        startRemoteW = remoteRect.width;
        const localFlex = parseFloat(localPane.style.flexGrow) || 1;
        const remoteFlex = parseFloat(remotePane.style.flexGrow) || 1;
        totalFlex = localFlex + remoteFlex;
        divider.classList.add('dragging');
        document.body.style.cursor = 'col-resize';
        document.addEventListener('mousemove', onMove);
        document.addEventListener('mouseup', onUp);
    });
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

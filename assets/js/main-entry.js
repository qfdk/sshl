// main-entry.js
// 应用程序入口文件，导入所有模块并初始化应用

// 导入所有管理器模块
import connectionManager from './connection-manager.js';
import uiManager from './ui-manager.js';

// 初始化应用程序
export function initializeApp(terminalManager, sessionManager) {
    // 设置全局变量和引用，使模块能够相互访问
    window.sessionManager = sessionManager;
    window.terminalManager = terminalManager;
    window.connectionManager = connectionManager;
    window.uiManager = uiManager;

    connectionManager.setupConnectionCancellation();

    // 初始化UI事件监听
    uiManager.initUIEvents();

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
}

export {
    connectionManager,
    uiManager
};

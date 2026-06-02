// ui-manager.js
// 处理UI相关功能

class UIManager {
    constructor() {
        this.loadingOverlay = null; // 加载遮罩元素
    }
    
    // 创建加载遮罩
    createLoadingOverlay(text = '连接中...') {
        // 如果已存在加载遮罩，先移除
        this.removeLoadingOverlay();

        // 创建新的加载遮罩
        this.loadingOverlay = document.createElement('div');
        this.loadingOverlay.className = 'loading-overlay';

        const spinner = document.createElement('div');
        spinner.className = 'spinner';

        const loadingText = document.createElement('div');
        loadingText.className = 'loading-text';
        loadingText.textContent = text;

        this.loadingOverlay.appendChild(spinner);
        this.loadingOverlay.appendChild(loadingText);

        document.body.appendChild(this.loadingOverlay);

        return this.loadingOverlay;
    }

    // 移除加载遮罩
    removeLoadingOverlay() {
        if (this.loadingOverlay && document.body.contains(this.loadingOverlay)) {
            document.body.removeChild(this.loadingOverlay);
            this.loadingOverlay = null;
        }
    }

    // 显示/隐藏文件管理器加载状态
    showFileManagerLoading(show) {
        const loadingOverlay = document.getElementById('file-manager-loading');
        if (loadingOverlay) {
            if (show) {
                loadingOverlay.classList.remove('hidden');
            } else {
                loadingOverlay.classList.add('hidden');
            }
        }
    }

    // 断开连接后强制回到终端标签（文件管理器无会话即失效，终端 placeholder 引导用户重新连接）。
    switchToTerminalTab() {
        if (window.activeTabId === 'terminal') return;

        document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
        document.querySelectorAll('.tab-pane').forEach(p => p.classList.remove('active'));

        const terminalTabBtn = document.querySelector('.tab[data-tab="terminal"]');
        if (terminalTabBtn) terminalTabBtn.classList.add('active');
        const terminalPane = document.getElementById('terminal-tab');
        if (terminalPane) terminalPane.classList.add('active');

        window.activeTabId = 'terminal';
    }

    // 显示/隐藏传输状态栏
    showTransferStatus(show) {
        const transferStatus = document.querySelector('.transfer-status');
        if (transferStatus) {
            if (show) {
                transferStatus.classList.add('active');
            } else {
                transferStatus.classList.remove('active');
            }
        }
    }
    
    // 更新连接状态
    updateConnectionStatus(connected, name = '') {
        const statusIndicator = document.querySelector('.status-indicator');
        const statusText = document.querySelector('.status-text');

        if (!statusIndicator || !statusText) {
            console.error('找不到状态指示器元素');
            return;
        }

        if (connected) {
            statusIndicator.classList.remove('offline');
            statusIndicator.classList.add('online');
            statusText.textContent = `已连接: ${name}`;
        } else {
            statusIndicator.classList.remove('online');
            statusIndicator.classList.add('offline');
            statusText.textContent = '未连接';
        }
    }
    
    // 更新服务器信息
    updateServerInfo(connected, serverInfo = {}) {
        // 更新主界面中的服务器信息
        const mainServerInfo = document.getElementById('main-server-info');
        if (mainServerInfo) {
            const indicator = mainServerInfo.querySelector('.server-indicator');
            const nameElement = mainServerInfo.querySelector('.server-name');

            if (connected && serverInfo.name) {
                indicator.classList.add('online');
                nameElement.textContent = `${serverInfo.name} (${serverInfo.host})`;
            } else {
                indicator.classList.remove('online');
                nameElement.textContent = '未连接';
            }
        }

        // 更新终端标签中的服务器信息
        const terminalServerInfo = document.getElementById('terminal-server-info');
        if (terminalServerInfo) {
            const indicator = terminalServerInfo.querySelector('.server-indicator');
            const nameElement = terminalServerInfo.querySelector('.server-name');

            if (connected && serverInfo.name) {
                indicator.classList.add('online');
                nameElement.textContent = `${serverInfo.name} (${serverInfo.host})`;
            } else {
                indicator.classList.remove('online');
                nameElement.textContent = '未连接';
            }
        }
    }
    
    // 处理连接项鼠标悬停
    handleItemHover(event) {
        // 只有在侧边栏折叠时显示工具提示
        const sidebar = document.querySelector('.sidebar');
        if (!sidebar || !sidebar.classList.contains('collapsed')) {
            return;
        }

        // 获取连接名称
        const connectionName = event.currentTarget.getAttribute('data-name');
        if (!connectionName) return;

        // 创建工具提示元素（如果不存在）
        let tooltip = document.getElementById('connection-tooltip');
        if (!tooltip) {
            tooltip = document.createElement('div');
            tooltip.id = 'connection-tooltip';
            tooltip.className = 'custom-tooltip';

            // 添加箭头元素
            const arrow = document.createElement('div');
            arrow.className = 'tooltip-arrow';
            tooltip.appendChild(arrow);

            document.body.appendChild(tooltip);
        }

        // 设置工具提示内容 - 确保不覆盖箭头
        // 清除箭头以外的现有内容
        const arrow = tooltip.querySelector('.tooltip-arrow');
        tooltip.innerHTML = '';
        tooltip.appendChild(arrow);

        // 添加文本作为单独的元素
        const textSpan = document.createElement('span');
        textSpan.textContent = connectionName;
        tooltip.appendChild(textSpan);

        // 计算位置
        const itemRect = event.currentTarget.getBoundingClientRect();
        const sidebarRect = sidebar.getBoundingClientRect();

        // 水平位置在侧边栏右侧，有间距
        tooltip.style.left = `${sidebarRect.right + 15}px`;

        // 垂直位置与项目中心对齐
        tooltip.style.top = `${itemRect.top + (itemRect.height / 2)}px`;
        tooltip.style.transform = 'translateY(-50%)'; // 使用transform垂直居中

        // 显示工具提示，轻微延迟以实现过渡效果
        setTimeout(() => {
            tooltip.classList.add('visible');
        }, 10);
    }

    // 处理连接项鼠标离开
    handleItemLeave(event) {
        const tooltip = document.getElementById('connection-tooltip');
        if (tooltip) {
            tooltip.classList.remove('visible');
        }
    }
    
    // 显示上下文菜单
    showContextMenu(x, y, items) {
        // 删除任何现有菜单
        const oldMenu = document.getElementById('context-menu');
        if (oldMenu) {
            document.body.removeChild(oldMenu);
        }

        // 创建新菜单
        const menu = document.createElement('div');
        menu.id = 'context-menu';
        menu.style.position = 'fixed';
        menu.style.left = `${x}px`;
        menu.style.top = `${y}px`;
        menu.style.backgroundColor = '#ffffff';
        menu.style.border = '1px solid #ddd';
        menu.style.borderRadius = '4px';
        menu.style.padding = '5px 0';
        menu.style.boxShadow = '0 2px 5px rgba(0,0,0,0.2)';
        menu.style.zIndex = '1000';

        // 添加菜单项
        items.forEach(item => {
            const menuItem = document.createElement('div');
            menuItem.textContent = item.label;
            menuItem.style.padding = '8px 12px';
            menuItem.style.cursor = 'pointer';
            menuItem.style.color = '#333';

            if (item.className) {
                menuItem.classList.add(item.className);
            }

            menuItem.addEventListener('click', () => {
                document.body.removeChild(menu);
                item.action();
            });

            menu.appendChild(menuItem);
        });

        // 添加到文档
        document.body.appendChild(menu);

        // 点击其他地方关闭菜单
        document.addEventListener('click', function closeMenu() {
            if (document.body.contains(menu)) {
                document.body.removeChild(menu);
            }
            document.removeEventListener('click', closeMenu);
        });
    }
    
    // 辅助函数：生成防抖函数
    debounce(func, wait) {
        let timeout;
        return function (...args) {
            const context = this;
            clearTimeout(timeout);
            timeout = setTimeout(() => func.apply(context, args), wait);
        };
    }
    
    // 初始化UI事件监听
    initUIEvents() {
        // 认证方式切换
        const authTypeSelect = document.getElementById('auth-type');
        if (authTypeSelect) {
            authTypeSelect.addEventListener('change', this.toggleAuthFields);
            this.toggleAuthFields(); // 初始设置
        }

        // 浏览私钥文件
        const browsePrivateKeyBtn = document.getElementById('browse-private-key');
        if (browsePrivateKeyBtn) {
            browsePrivateKeyBtn.addEventListener('click', async (e) => {
                e.preventDefault();
                const result = await window.api.dialog.selectFile();
                if (!result.canceled) {
                    document.getElementById('conn-private-key-path').value = result.filePaths[0];
                }
            });
        }
        
        // 标签切换
        const tabs = document.querySelectorAll('.tab');
        tabs.forEach(tab => {
            tab.addEventListener('click', this.debounce(function () {
                const tabId = tab.getAttribute('data-tab');

                // 避免切换到相同标签
                if (tabId === window.activeTabId) {
                    return;
                }

                // 设置标志以防止多次操作
                window.terminalManager.isTabSwitching = true;

                // 未连接也允许切换：终端显示 placeholder 引导重连，文件管理器仅本地可浏览。

                tabs.forEach(t => t.classList.remove('active'));
                document.querySelectorAll('.tab-pane').forEach(p => p.classList.remove('active'));

                tab.classList.add('active');
                document.getElementById(`${tabId}-tab`).classList.add('active');

                // 更新当前活动标签
                window.activeTabId = tabId;

                // 如果切换到文件管理器，总是重新初始化文件列表（未连接时仅本地）
                if (tabId === 'file-manager') {
                    // 显示文件管理器加载状态
                    window.uiManager.showFileManagerLoading(true);
                    // 清除文件管理器缓存
                    window.fileManager.clearFileManagerCache();

                    // 延迟初始化以确保UI已更新（currentSessionId 为 null 时只加载本地面板）
                    setTimeout(() => {
                        // 确保使用最新的会话ID
                        window.fileManager.initFileManager(window.currentSessionId);
                        window.fileManager.fileManagerInitialized = true;
                    }, 100);
                }

                // 如果切换到终端标签，调整终端大小，但不要刷新终端内容
                if (tabId === 'terminal' && window.terminalManager.activeTerminal) {
                    window.terminalManager.ensureTerminalVisible();
                    setTimeout(() => {
                        window.terminalManager.resizeTerminal();
                        window.terminalManager.isTabSwitching = false; // 重置标志，一切完成后
                    }, 50);
                } else {
                    window.terminalManager.isTabSwitching = false; // 重置其他标签的标志
                }
            }, 100)); // 减少防抖延迟到100毫秒提高响应性
        });
        
        // 侧边栏折叠/展开
        const sidebarToggle = document.getElementById('toggle-sidebar');
        const sidebar = document.querySelector('.sidebar');
        if (sidebarToggle && sidebar) {
            sidebarToggle.addEventListener('click', () => {
                sidebar.classList.toggle('collapsed');

                // 更新箭头方向
                const collapsed = sidebar.classList.contains('collapsed');
                sidebarToggle.innerHTML = window.Icons.svg(collapsed ? 'chevron-right' : 'chevron-left', 16);

                // 侧边栏展开时，隐藏任何可能显示的工具提示
                if (!sidebar.classList.contains('collapsed')) {
                    const tooltip = document.getElementById('connection-tooltip');
                    if (tooltip) {
                        tooltip.classList.remove('visible');
                    }
                }

                // 侧边栏变化后调整终端大小
                setTimeout(() => window.terminalManager.resizeTerminal(), 300);
            });
        }
        
        // 新建连接
        const newConnectionBtn = document.getElementById('new-connection-btn');
        const connectionDialog = document.getElementById('connection-dialog');
        if (newConnectionBtn && connectionDialog) {
            newConnectionBtn.addEventListener('click', () => {
                connectionDialog.classList.add('active');
                // 重置认证方式为密码，并触发UI更新
                document.getElementById('auth-type').value = 'password';
                this.toggleAuthFields();
                // 确保是新建模式，清除编辑标记
                const form = document.getElementById('connection-form');
                if (form) {
                    delete form.dataset.editingId;
                }
                // 重置提交按钮文本
                const submitBtn = document.getElementById('connection-submit-btn');
                if (submitBtn) {
                    submitBtn.innerHTML = `${window.Icons.svg('arrow-right', 16)}连接`;
                }
            });
        }

        // 取消连接
        const cancelConnectionBtn = document.getElementById('cancel-connection');
        const connectionForm = document.getElementById('connection-form');
        if (cancelConnectionBtn && connectionForm && connectionDialog) {
            cancelConnectionBtn.addEventListener('click', () => {
                connectionDialog.classList.remove('active');
                connectionForm.reset();
                // 重置认证方式为密码，并触发UI更新
                document.getElementById('auth-type').value = 'password';
                this.toggleAuthFields();
                // 清除编辑模式标记
                delete connectionForm.dataset.editingId;
                // 重置提交按钮文本
                const submitBtn = document.getElementById('connection-submit-btn');
                if (submitBtn) {
                    submitBtn.innerHTML = `${window.Icons.svg('arrow-right', 16)}连接`;
                }
            });
        }

        // 提交连接表单
        if (connectionForm) {
            connectionForm.addEventListener('submit', (e) => window.connectionManager.handleConnectionFormSubmit(e));
        }
        
        // 本地文件浏览按钮
        const browseLocalBtn = document.getElementById('browse-local');
        if (browseLocalBtn) {
            browseLocalBtn.addEventListener('click', async () => {
                await window.fileManager.loadLocalFiles(null); // 传递 null 会触发目录选择对话框
            });
        }

        // 远程路径导航按钮
        const goRemotePathBtn = document.getElementById('go-remote-path');
        if (goRemotePathBtn) {
            goRemotePathBtn.addEventListener('click', function () {
                const path = document.getElementById('remote-path').value;
                if (path) {
                    // 防止重复点击
                    this.disabled = true;
                    window.fileManager.loadRemoteFiles(path).finally(() => {
                        this.disabled = false;
                    });
                }
            });
        }

        // 本地刷新按钮
        const localRefreshBtn = document.getElementById('local-refresh');
        if (localRefreshBtn) {
            localRefreshBtn.addEventListener('click', function () {
                const path = document.getElementById('local-path').value;
                if (path) {
                    // 防止重复点击
                    this.disabled = true;
                    window.fileManager.loadLocalFiles(path).finally(() => {
                        this.disabled = false;
                    });
                }
            });
        }

        // 远程刷新按钮
        const remoteRefreshBtn = document.getElementById('remote-refresh');
        if (remoteRefreshBtn) {
            remoteRefreshBtn.addEventListener('click', function () {
                const path = document.getElementById('remote-path').value;
                if (path) {
                    // 防止重复点击
                    this.disabled = true;
                    window.fileManager.loadRemoteFiles(path).finally(() => {
                        this.disabled = false;
                    });
                }
            });
        }

        // 本地主目录按钮
        const localHomeBtn = document.getElementById('local-home');
        if (localHomeBtn) {
            localHomeBtn.addEventListener('click', async function () {
                this.disabled = true;
                try {
                    const home = await window.api.file.getHomeDir();
                    if (home) await window.fileManager.loadLocalFiles(home);
                } catch (e) {
                    console.error('获取本地主目录失败:', e);
                } finally {
                    this.disabled = false;
                }
            });
        }

        // 远程主目录按钮
        const remoteHomeBtn = document.getElementById('remote-home');
        if (remoteHomeBtn) {
            remoteHomeBtn.addEventListener('click', async function () {
                if (!window.currentSessionId) return;
                this.disabled = true;
                try {
                    const sessionId = String(window.currentSessionId);
                    const result = await window.api.ssh.execute(sessionId, 'echo "$HOME"');
                    const home = (result && (result.output ?? result.data ?? result))?.toString().trim();
                    if (home) await window.fileManager.loadRemoteFiles(home);
                } catch (e) {
                    console.error('获取远程主目录失败:', e);
                } finally {
                    this.disabled = false;
                }
            });
        }
    }
    
    // 切换认证方式显示/隐藏相关字段
    toggleAuthFields() {
        const authTypeSelect = document.getElementById('auth-type');
        const passwordAuthFields = document.querySelector('.auth-password');
        const privateKeyAuthFields = document.querySelectorAll('.auth-key');
        
        if (!authTypeSelect || !passwordAuthFields) return;
        
        const authType = authTypeSelect.value;

        if (authType === 'password') {
            passwordAuthFields.classList.remove('hidden');
            privateKeyAuthFields.forEach(field => field.classList.add('hidden'));
            // 清除私钥相关字段
            document.getElementById('conn-private-key-path').value = '';
            document.getElementById('conn-passphrase').value = '';
        } else {
            passwordAuthFields.classList.add('hidden');
            privateKeyAuthFields.forEach(field => field.classList.remove('hidden'));
            // 清除密码字段
            document.getElementById('conn-password').value = '';

            // 自动设置默认私钥路径为 ~/.ssh/id_rsa
            if (window.api && window.api.file && window.api.file.getHomeDir) {
                window.api.file.getHomeDir()
                    .then(homeDir => {
                        // 确定正确的路径分隔符
                        const separator = homeDir.includes('\\') ? '\\' : '/';

                        // 使用正确的分隔符构建路径
                        let defaultPrivateKeyPath;
                        if (separator === '\\') {
                            // Windows 风格路径
                            defaultPrivateKeyPath = homeDir + '\\.ssh\\id_rsa';
                        } else {
                            // Unix 风格路径
                            defaultPrivateKeyPath = homeDir + '/.ssh/id_rsa';
                        }

                        document.getElementById('conn-private-key-path').value = defaultPrivateKeyPath;
                    })
                    .catch(err => console.error('获取用户主目录失败:', err));
            }
        }
    }
    
    // 设置回调函数，用于输入框回车键处理
    setupEnterKeyHandler(elementId, loadFunction) {
        const inputElement = document.getElementById(elementId);

        if (inputElement) {
            inputElement.addEventListener('keydown', async (e) => {
                if (e.key === 'Enter') {
                    e.preventDefault();
                    const path = inputElement.value;

                    if (path) {
                        await loadFunction(path);
                    }
                }
            });
        }
    }
}

// 导出单例实例
const uiManager = new UIManager();
export default uiManager;
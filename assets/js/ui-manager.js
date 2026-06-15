// ui-manager.js
// 处理UI相关功能

class UIManager {
    constructor() {
        this.loadingOverlay = null; // 加载遮罩元素
        // 「填充密码」按钮状态：有保存连接(available) + 终端正显示密码提示时才显示
        this._fillPwd = { available: false, promptVisible: false, showingFeedback: false };
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

        // 同步「填充密码」按钮状态（连接是否密码模式 + 当前是否密码提示）
        this.syncFillPasswordState();
    }

    // 判定按钮可用性并据当前终端内容刷新一次。在连接/切换会话（updateServerInfo）时调用。
    // 只要有保存连接即可用 —— su/sudo 在私钥登录的服务器上同样需要密码，账号密码存在该连接名下。
    syncFillPasswordState() {
        const sessionId = window.currentSessionId;
        const connectionId = sessionId
            ? window.sessionManager?.getSession(sessionId)?.connectionId
            : null;
        this._fillPwd.available = !!connectionId;
        // 切换会话后立即按当前终端内容判定一次（避免等到下次渲染才出现）
        const term = window.terminalManager?.activeTerminal;
        this._fillPwd.promptVisible = !!window.terminalManager?.isPasswordPromptVisible?.(term);
        this._applyFillPasswordBtn();
    }

    // 终端渲染时调用：更新「当前是否显示密码提示」
    setFillPasswordPromptVisible(visible) {
        if (this._fillPwd.promptVisible === visible) return;
        this._fillPwd.promptVisible = visible;
        this._applyFillPasswordBtn();
    }

    // 应用按钮组显隐：可用 + 密码提示可见时显示；反馈期间强制可见。隐藏时一并收起菜单。
    _applyFillPasswordBtn() {
        const group = document.getElementById('fill-password-group');
        if (!group) return;
        const visible = this._fillPwd.showingFeedback
            || (this._fillPwd.available && this._fillPwd.promptVisible);
        group.hidden = !visible;
        if (!visible) {
            const menu = document.getElementById('fill-password-menu');
            if (menu) menu.hidden = true;
        }
    }

    // 填充指定密码（kind: 省略/'password' = 连接主密码；'acct:<账号>' = 账号密码），自动回车
    async doFillPassword(kind) {
        const sessionId = window.currentSessionId;
        if (!sessionId) return;
        try {
            const r = await window.api.ssh.fillPassword(sessionId, kind);
            if (r && r.success) {
                this.flashFillPasswordDone();
                const term = window.terminalManager?.activeTerminal;
                if (term) { try { term.focus(); } catch (_) {} }
            } else {
                alert(`填充密码失败: ${r ? r.error : '未知错误'}`);
            }
        } catch (e) {
            console.error('填充密码失败:', e);
            alert(`填充密码失败: ${e.message || e}`);
        }
    }

    // 主键点击：只有一个候选直接填（sudo 常见），多个候选则打开菜单让用户选
    async fillPasswordPrimary() {
        const connectionId = window.sessionManager?.getSession(window.currentSessionId)?.connectionId;
        if (!connectionId) return;
        let info = null;
        try { info = await window.api.cred.list(connectionId); } catch (_) {}
        const accounts = (info && info.accounts) || [];
        const hasPwd = !!(info && info.hasPassword);
        if ((hasPwd ? 1 : 0) + accounts.length <= 1) {
            if (hasPwd) return this.doFillPassword('password');
            if (accounts.length === 1) return this.doFillPassword('acct:' + accounts[0]);
            return this.openFillPasswordMenu(); // 无任何已存密码 → 打开菜单（含新增表单）
        }
        return this.openFillPasswordMenu();
    }

    // 反馈「填充发送完毕」1.5s（提示行此时已消失，反馈期间强制按钮可见）
    flashFillPasswordDone() {
        const btn = document.getElementById('fill-password-btn');
        const label = btn && btn.querySelector('.fill-password-label');
        if (btn) btn.classList.add('filled');
        if (label) label.textContent = '填充发送完毕';
        this._fillPwd.showingFeedback = true;
        this._applyFillPasswordBtn();
        clearTimeout(this._fillPwd._t);
        this._fillPwd._t = setTimeout(() => {
            if (btn) btn.classList.remove('filled');
            if (label) label.textContent = '填充密码';
            this._fillPwd.showingFeedback = false;
            this._applyFillPasswordBtn();
        }, 1500);
    }

    async openFillPasswordMenu() {
        const menu = document.getElementById('fill-password-menu');
        const connectionId = window.sessionManager?.getSession(window.currentSessionId)?.connectionId;
        if (!menu || !connectionId) return;
        await this.buildFillPasswordMenu(menu, connectionId);
        menu.hidden = false;
    }

    // 构建下拉：当前连接密码 + 各账号（可删）+ 新增账号密码表单
    async buildFillPasswordMenu(menu, connectionId) {
        menu.innerHTML = '';
        let info = null;
        try { info = await window.api.cred.list(connectionId); } catch (e) { console.error('获取凭据失败:', e); }
        const accounts = (info && info.accounts) || [];
        const hasPwd = !!(info && info.hasPassword);

        const addRow = (name, tag, onClick, onDelete) => {
            const item = document.createElement('div');
            item.className = 'fill-password-menu-item';
            const left = document.createElement('span');
            left.className = 'pw-name';
            left.textContent = name;
            item.appendChild(left);
            if (tag) {
                const t = document.createElement('span');
                t.className = 'pw-tag';
                t.textContent = tag;
                item.appendChild(t);
            }
            if (onDelete) {
                const del = document.createElement('button');
                del.className = 'pw-del';
                del.type = 'button';
                del.textContent = '×';
                del.title = '删除';
                del.addEventListener('click', (e) => { e.stopPropagation(); onDelete(); });
                item.appendChild(del);
            }
            item.addEventListener('click', onClick);
            menu.appendChild(item);
        };

        if (hasPwd) {
            addRow('当前连接密码', '主', () => { menu.hidden = true; this.doFillPassword('password'); });
        }
        accounts.forEach((acct) => {
            addRow(acct, '', () => { menu.hidden = true; this.doFillPassword('acct:' + acct); }, async () => {
                try {
                    await window.api.cred.delete(connectionId, acct);
                    await this.buildFillPasswordMenu(menu, connectionId);
                } catch (e) { alert(`删除失败: ${e.message || e}`); }
            });
        });

        if (!hasPwd && accounts.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'fill-password-menu-empty';
            empty.textContent = '暂无保存的密码，下面新增';
            menu.appendChild(empty);
        }

        const sep = document.createElement('div');
        sep.className = 'fill-password-menu-sep';
        menu.appendChild(sep);

        const form = document.createElement('div');
        form.className = 'fill-password-add';
        form.addEventListener('click', (e) => e.stopPropagation());
        const acctInput = document.createElement('input');
        acctInput.className = 'pw-acct';
        acctInput.type = 'text';
        acctInput.placeholder = '账号';
        const pwInput = document.createElement('input');
        pwInput.type = 'password';
        pwInput.placeholder = '密码';
        const save = document.createElement('button');
        save.type = 'button';
        save.textContent = '+';
        save.title = '保存账号密码';
        const doSave = async () => {
            const account = acctInput.value.trim();
            if (!account) { acctInput.focus(); return; }
            try {
                await window.api.cred.set(connectionId, account, pwInput.value);
                await this.buildFillPasswordMenu(menu, connectionId);
            } catch (e) { alert(`保存失败: ${e.message || e}`); }
        };
        save.addEventListener('click', (e) => { e.stopPropagation(); doSave(); });
        pwInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); doSave(); } });
        form.appendChild(acctInput);
        form.appendChild(pwInput);
        form.appendChild(save);
        menu.appendChild(form);
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

        // 填充密码：分体按钮（主键智能填充 + 下拉选择账号/管理），自动回车，常用于 sudo / su
        const fillPwGroup = document.getElementById('fill-password-group');
        const fillPwBtn = document.getElementById('fill-password-btn');
        const fillPwCaret = document.getElementById('fill-password-menu-btn');
        const fillPwMenu = document.getElementById('fill-password-menu');
        if (fillPwBtn) {
            const iconSlot = fillPwBtn.querySelector('.fill-password-icon');
            if (iconSlot && window.Icons) iconSlot.innerHTML = window.Icons.svg('key-round', 15, 2);
            fillPwBtn.addEventListener('click', () => window.uiManager.fillPasswordPrimary());
        }
        if (fillPwCaret && fillPwMenu) {
            fillPwCaret.addEventListener('click', async (e) => {
                e.stopPropagation();
                if (!fillPwMenu.hidden) { fillPwMenu.hidden = true; return; }
                await window.uiManager.openFillPasswordMenu();
            });
            // 点击按钮组以外区域关闭菜单
            document.addEventListener('click', (e) => {
                if (fillPwGroup && !fillPwGroup.contains(e.target)) fillPwMenu.hidden = true;
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
// connection-manager.js
// 处理连接相关功能

class ConnectionManager {
    constructor() {
        this.isConnecting = false; // 连接中状态标志
    }
    
    // 加载连接列表
    async loadConnections() {
        try {
            if (!window.api || !window.api.config) {
                console.error('API未初始化，无法加载连接');
                return;
            }

            const connections = await window.api.config.getConnections();
            const connectionList = document.getElementById('connection-list');

            connectionList.innerHTML = '';

            if (connections && connections.length > 0) {
                connections.forEach(connection => {
                    // 检查该连接是否有活跃会话（不论是否在前台）
                    const existingSessionInfo = window.sessionManager.getSessionByConnectionId(connection.id);
                    const isConnected = existingSessionInfo !== null;
                    // 是否是当前前台会话
                    const isActive = isConnected &&
                        existingSessionInfo.sessionId === window.currentSessionId;

                    const statusClass = isConnected ? 'online' : 'offline';

                    const item = document.createElement('div');
                    item.className = 'connection-item';
                    item.setAttribute('data-id', connection.id);
                    item.setAttribute('data-active', isActive ? 'true' : 'false');
                    item.setAttribute('data-connected', isConnected ? 'true' : 'false');
                    // 储存名称，但不使用title属性（会显示原生工具提示）
                    item.setAttribute('data-name', connection.name);

                    const disconnectBtn = isConnected ? `
                            <button class="icon-button disconnect-connection" data-session-id="${existingSessionInfo.sessionId}" title="断开连接">
                                ${window.Icons.svg('power', 14, 2.5)}
                            </button>` : '';

                    item.innerHTML = `
                        <div class="connection-status-indicator ${statusClass}"></div>
                        <div class="connection-name">${connection.name}</div>
                        <div class="connection-actions">
                            ${disconnectBtn}
                            <button class="icon-button edit-connection" data-id="${connection.id}" title="${isConnected ? '断开后才能编辑' : '编辑连接'}"${isConnected ? ' disabled' : ''}>
                                ${window.Icons.svg('square-pen', 14, 2.5)}
                            </button>
                            <button class="icon-button delete-connection" data-id="${connection.id}" title="${isConnected ? '断开后才能删除' : '删除连接'}"${isConnected ? ' disabled' : ''}>
                                ${window.Icons.svg('trash-2', 14, 2.5)}
                            </button>
                        </div>
                    `;

                    // 添加双击事件
                    item.addEventListener('dblclick', async () => {
                        await this.connectToSaved(connection.id);
                    });

                    // 添加鼠标悬停事件，用于显示自定义工具提示
                    item.addEventListener('mouseenter', window.uiManager.handleItemHover);
                    item.addEventListener('mouseleave', window.uiManager.handleItemLeave);

                    connectionList.appendChild(item);
                });
            } else {
                connectionList.innerHTML = '<div class="no-connections">没有保存的连接</div>';
            }
        } catch (error) {
            console.error('加载连接失败:', error);
        }
    }
    
    // 更新活跃连接项：data-active 只标记当前前台会话；指示器 online/offline 基于该连接是否有活跃会话
    // 已连接但还没断开按钮的项动态注入按钮；已断开则移除按钮
    updateActiveConnectionItem(activeConnectionId) {
        const connectionItems = document.querySelectorAll('.connection-item');

        connectionItems.forEach(item => {
            const itemConnectionId = item.getAttribute('data-id');
            const isForeground = itemConnectionId === activeConnectionId;
            const sessionInfo = window.sessionManager.getSessionByConnectionId(itemConnectionId);
            const isConnected = sessionInfo !== null;

            item.setAttribute('data-active', isForeground ? 'true' : 'false');
            item.setAttribute('data-connected', isConnected ? 'true' : 'false');

            const indicator = item.querySelector('.connection-status-indicator');
            if (indicator) {
                if (isConnected) {
                    indicator.classList.remove('offline');
                    indicator.classList.add('online');
                } else {
                    indicator.classList.remove('online');
                    indicator.classList.add('offline');
                }
            }

            // 同步断开按钮
            const actions = item.querySelector('.connection-actions');
            const existingBtn = item.querySelector('.disconnect-connection');
            if (isConnected && !existingBtn && actions) {
                const btn = document.createElement('button');
                btn.className = 'icon-button disconnect-connection';
                btn.setAttribute('data-session-id', sessionInfo.sessionId);
                btn.setAttribute('title', '断开连接');
                btn.innerHTML = window.Icons.svg('power', 14, 2.5);
                actions.insertBefore(btn, actions.firstChild);
            } else if (!isConnected && existingBtn) {
                existingBtn.remove();
            } else if (isConnected && existingBtn) {
                // 更新 sessionId（可能因重连而变化）
                existingBtn.setAttribute('data-session-id', sessionInfo.sessionId);
            }

            // 同步编辑/删除按钮的禁用状态（连接中不可操作）
            const editBtn = item.querySelector('.edit-connection');
            const deleteBtn = item.querySelector('.delete-connection');
            if (editBtn) {
                editBtn.disabled = isConnected;
                editBtn.setAttribute('title', isConnected ? '断开后才能编辑' : '编辑连接');
            }
            if (deleteBtn) {
                deleteBtn.disabled = isConnected;
                deleteBtn.setAttribute('title', isConnected ? '断开后才能删除' : '删除连接');
            }
        });
    }
    
    
    // 切换到现有会话
    async switchToSession(connectionId) {
        // 获取会话信息
        const sessionInfo = window.sessionManager.getSessionByConnectionId(connectionId);

        // 如果是当前会话，直接返回
        if (sessionInfo && window.currentSessionId === sessionInfo.sessionId) {
            return true;
        }

        // 清除文件管理器缓存
        window.fileManager.clearFileManagerCache();

        try {
            // 获取会话信息
            const sessionInfo = window.sessionManager.getSessionByConnectionId(connectionId);
            if (!sessionInfo) {
                console.error(`[switchToSession] 找不到连接ID: ${connectionId} 的会话`);
                return false;
            }

            // 如果是当前会话，直接返回
            if (window.currentSessionId === sessionInfo.sessionId) {
                return true;
            }

            // 检查会话是否有效。Tauri 后端独立管理 session 生命周期，前端没有 stream 字段；
            // 只在 sessionManager 完全缺失时视为失效。后端实际是否存活由 activateSession 检测。
            const session = sessionInfo.session;
            if (!session) {
                // 清理旧会话及其失效的 xterm 实例
                if (sessionInfo.sessionId) {
                    window.sessionManager.removeSession(sessionInfo.sessionId);
                    window.terminalManager.disposeTerminalInstance(sessionInfo.sessionId);
                }

                // 从配置获取连接信息
                const connections = await window.api.config.getConnections();
                const connection = connections.find(c => c.id === connectionId);
                if (!connection) {
                    console.error('[switchToSession] 找不到连接信息');
                    return false;
                }

                // 重新连接
                try {
                    const result = await window.api.ssh.connect(connection);
                    if (result.success) {
                        // 更新会话ID
                        window.currentSessionId = result.sessionId;

                        // 创建该 session 的 xterm 实例（其他后台 session 的 host 会被隐藏，但不销毁）
                        const terminalInfo = await window.terminalManager.initTerminal(result.sessionId, null, false);
                        // 保存到会话管理器
                        window.sessionManager.addSession(result.sessionId, connectionId, {
                            term: terminalInfo.term,
                            buffer: '',
                            name: connection.name
                        });

                        // 原子激活：activate 返回激活瞬间的缓冲快照并开始 emit，避免
                        // fetch→activate 间隙丢掉慢 MOTD 后到达的首个 PS1 提示符
                        try {
                            const activateResult = await window.api.ssh.activateSession(result.sessionId);
                            if (activateResult?.success && activateResult.buffer && terminalInfo?.term) {
                                terminalInfo.term.write(activateResult.buffer);
                            }
                        } catch (err) {
                            console.warn(`[switchToSession reconnect] activate 失败:`, err);
                        }

                        // 更新UI
                        window.uiManager.updateConnectionStatus(true, connection.name);
                        window.uiManager.updateServerInfo(true, {
                            name: connection.name,
                            host: connection.host
                        });
                        this.updateActiveConnectionItem(connectionId);

                        // 重置文件管理器状态
                        window.fileManager.fileManagerInitialized = false;
                        
                        // 如果当前活动标签是文件管理器，立即初始化它
                        const activeTab = document.querySelector('.tab.active');
                        if (activeTab && activeTab.getAttribute('data-tab') === 'file-manager') {
                            // 显示文件管理器加载状态
                            window.uiManager.showFileManagerLoading(true);
                            // 延迟初始化以确保UI已更新
                            setTimeout(() => {
                                window.fileManager.initFileManager(result.sessionId);
                                window.fileManager.fileManagerInitialized = true;
                            }, 100);
                        }

                        return true;
                    } else {
                        console.error('[switchToSession] 重新连接失败', result.error);
                        return false;
                    }
                } catch (error) {
                    console.error('[switchToSession] 重新连接出错:', error);
                    return false;
                }
            }

            // 标记上一个会话为非活跃（数据仍会写入其 xterm，便于切回时即时可见）
            if (window.currentSessionId && window.currentSessionId !== sessionInfo.sessionId) {
                window.sessionManager.setSessionActive(window.currentSessionId, false);
            }

            window.currentSessionId = sessionInfo.sessionId;
            window.sessionManager.setSessionActive(sessionInfo.sessionId, true);

            // 全局 ssh:data / ssh:closed 监听器只注册一次（按 sessionId 路由）
            this.setupSSHHandlers();

            // 切换到该 session 的 xterm 实例：已存在则复用（保留 scrollback），否则新建
            const terminalResult = await window.terminalManager.initTerminal(
                sessionInfo.sessionId,
                sessionInfo.session,
                false
            );

            if (!terminalResult) {
                throw new Error('终端初始化失败');
            }

            // 仅新建终端时拉缓冲区回放；复用则跳过，避免内容重复
            if (terminalResult.isNew) {
                try {
                    const bufferResult = await window.api.ssh.getSessionBuffer(sessionInfo.sessionId);
                    if (bufferResult?.success && bufferResult.buffer) {
                        terminalResult.term.write(bufferResult.buffer);
                    }
                } catch (err) {
                    console.warn(`[switchToSession] 加载缓冲区数据失败:`, err);
                }
            }
            try {
                const activateResult = await window.api.ssh.activateSession(sessionInfo.sessionId);
                if (activateResult?.sessionId && activateResult.sessionId !== sessionInfo.sessionId) {
                    window.currentSessionId = activateResult.sessionId;
                    window.sessionManager.updateSessionId(sessionInfo.sessionId, activateResult.sessionId);
                    sessionInfo.sessionId = activateResult.sessionId;
                }
            } catch (err) {
                console.warn(`[switchToSession] 在后端激活会话失败:`, err);
            }

            // 异步加载连接信息和更新UI
            window.api.config.getConnections().then(connections => {
                const connection = connections.find(c => c.id === connectionId);
                if (connection) {
                    // 更新UI状态
                    window.uiManager.updateConnectionStatus(true, connection.name);
                    window.uiManager.updateServerInfo(true, {
                        name: connection.name,
                        host: connection.host
                    });
                    this.updateActiveConnectionItem(connectionId);
                }
            }).catch(err => {
                console.error('获取连接信息失败:', err);
            });

            // 重置文件管理器状态，确保使用新连接重新初始化
            window.fileManager.fileManagerInitialized = false;

            // 如果当前活动标签是文件管理器，立即初始化它
            const activeTab = document.querySelector('.tab.active');
            if (activeTab && activeTab.getAttribute('data-tab') === 'file-manager') {
                // 显示文件管理器加载状态
                window.uiManager.showFileManagerLoading(true);

                // 等待终端初始化完成后，再初始化文件管理器
                setTimeout(() => {
                    // 确保使用最新的会话ID
                    const currentSessionId = window.currentSessionId;
                    window.fileManager.initFileManager(currentSessionId);
                    window.fileManager.fileManagerInitialized = true;
                }, 100);
            }

            // 确保终端大小正确，但使用延迟调整避免高CPU使用
            setTimeout(() => window.terminalManager.resizeTerminal(), 150);

            // 最后更新活跃连接指示器，确保所有状态都已更新
            setTimeout(() => {
                this.updateActiveConnectionItem(connectionId);
            }, 200);

            return true;
        } catch (error) {
            console.error('切换会话失败:', error);
            return false;
        }
    }
    
    // 显示编辑连接对话框
    showEditConnectionDialog(connection) {
        // 填充表单字段
        document.getElementById('conn-name').value = connection.name || '';
        document.getElementById('conn-host').value = connection.host || '';
        document.getElementById('conn-port').value = connection.port || 22;
        document.getElementById('conn-username').value = connection.username || '';
        
        // 设置认证类型 —— 后端不持久化 authType，根据 privateKey 字段推断
        const inferredAuthType = connection.authType
            || (connection.privateKey ? 'privateKey' : 'password');
        const authTypeSelect = document.getElementById('auth-type');
        authTypeSelect.value = inferredAuthType;

        // 触发认证类型变更事件，显示正确的字段
        authTypeSelect.dispatchEvent(new Event('change'));

        // 根据认证类型填充相应字段
        if (inferredAuthType === 'password') {
            document.getElementById('conn-password').value = connection.password || '';
        } else if (inferredAuthType === 'privateKey') {
            document.getElementById('conn-private-key-path').value = connection.privateKey || '';
            document.getElementById('conn-passphrase').value = connection.passphrase || '';
        }
        
        // 设置保存密码选项
        document.getElementById('conn-save-password').checked = !!(connection.password || connection.passphrase);
        
        // 存储正在编辑的连接ID，用于更新而不是创建新连接
        const form = document.getElementById('connection-form');
        form.dataset.editingId = connection.id;
        
        // 更新提交按钮文本
        const submitBtn = document.getElementById('connection-submit-btn');
        if (submitBtn) {
            submitBtn.innerHTML = `${window.Icons.svg('save', 16)}保存`;
        }
        
        // 显示对话框
        document.getElementById('connection-dialog').classList.add('active');
        
        // 聚焦到名称字段
        setTimeout(() => {
            document.getElementById('conn-name').focus();
        }, 100);
    }
    
    // 连接到保存的连接
    async connectToSaved(id) {
        // 如果已经在连接中，则忽略
        if (this.isConnecting) return;

        try {
            if (!window.api) {
                alert('API未初始化，请重启应用');
                return;
            }

            // 如果双击的是当前正在显示的会话，无需做任何切换/清空操作
            const existing = window.sessionManager.getSessionByConnectionId(id);
            if (existing && existing.sessionId === window.currentSessionId) {
                return;
            }

            const connections = await window.api.config.getConnections();
            const connection = connections.find(c => c.id === id);

            if (!connection) {
                console.error('找不到连接信息');
                return;
            }

            // 尝试切换到现有会话
            const sessionInfo = window.sessionManager.getSessionByConnectionId(connection.id);

            if (sessionInfo) {
                // 确保会话被标记为活跃状态
                if (sessionInfo.session && !sessionInfo.session.active) {
                    sessionInfo.session.active = true;
                    window.sessionManager.updateSession(sessionInfo.sessionId, {active: true});
                }

                // 使用新的切换功能
                const switchResult = await this.switchToSession(connection.id);

                if (switchResult) {
                    // 更新服务器信息显示
                    window.uiManager.updateServerInfo(true, {
                        name: connection.name,
                        host: connection.host
                    });
                    return;
                } else {
                    console.warn('会话切换失败，尝试建立新连接');
                }
            }

            // 如果没有现有会话或切换失败，建立新连接
            this.isConnecting = true;
            window.uiManager.createLoadingOverlay('正在连接服务器...');

            const result = await window.api.ssh.connect(connection);

            if (result && result.success) {
                window.currentSessionId = result.sessionId;

                // 不再 re-save 连接：后端 StoredConnection 不持久化 sessionId，这次回存毫无用处，
                // 反而会因传入对象缺少 password 字段把 has_password 标志冲成 false（密码仍在 secrets 库）。
                const terminalInfo = await window.terminalManager.initTerminal(
                    result.sessionId,
                    null,
                    false
                );

                if (terminalInfo) {
                    window.sessionManager.addSession(result.sessionId, connection.id, {
                        term: terminalInfo.term,
                        buffer: '',
                        name: connection.name
                    });
                }

                // 原子激活：activate 返回激活瞬间的缓冲快照并开始 emit，避免
                // fetch→activate 间隙丢掉慢 MOTD 后到达的首个 PS1 提示符
                try {
                    const activateResult = await window.api.ssh.activateSession(result.sessionId);
                    if (activateResult?.success && activateResult.buffer && terminalInfo?.term) {
                        terminalInfo.term.write(activateResult.buffer);
                    }
                } catch (err) {
                    console.warn(`[连接] 激活会话失败:`, err);
                }

                // 更新状态
                window.uiManager.updateConnectionStatus(true, connection.name);
                // 更新服务器信息
                window.uiManager.updateServerInfo(true, {
                    name: connection.name,
                    host: connection.host
                });
                this.updateActiveConnectionItem(connection.id);

                // 更新连接列表（已包含活跃状态更新）
                await this.loadConnections();

                // 重置文件管理器状态
                window.fileManager.fileManagerInitialized = false;

                // 获取当前激活的标签
                const currentActiveTab = document.querySelector('.tab.active');

                // 如果文件管理器标签处于活动状态，现在初始化它
                if (currentActiveTab && currentActiveTab.getAttribute('data-tab') === 'file-manager') {
                    // 显示加载状态
                    window.uiManager.showFileManagerLoading(true);

                    // 短暂延迟以确保会话准备就绪
                    setTimeout(() => {
                        window.fileManager.initFileManager(result.sessionId);
                        window.fileManager.fileManagerInitialized = true;
                    }, 100);
                }

                // 保持当前激活的标签类型
                if (currentActiveTab) {
                    currentActiveTab.click();
                }
            } else {
                alert(`连接失败: ${result ? result.error || 'unknown error' : 'unknown error'}`);
            }
        } catch (error) {
            console.error('连接错误:', error);
            alert(`连接错误: ${error ? error.message || '未知错误' : '未知错误'}`);
        } finally {
            this.isConnecting = false;
            window.uiManager.removeLoadingOverlay();
        }
    }
    
    // 处理连接表单提交
    async handleConnectionFormSubmit(e) {
        e.preventDefault();

        const form = e.target;
        const editingId = form.dataset.editingId;
        
        // 如果是编辑模式，只保存不连接
        if (editingId) {
            await this.handleEditConnection(editingId);
            return;
        }

        // 如果已经在连接中，则忽略
        if (this.isConnecting) return;

        try {
            this.isConnecting = true;
            window.uiManager.createLoadingOverlay('正在连接服务器...');

            const authType = document.getElementById('auth-type').value;
            const savePassword = document.getElementById('conn-save-password').checked;

            const connectionDetails = {
                name: document.getElementById('conn-name').value,
                host: document.getElementById('conn-host').value,
                port: parseInt(document.getElementById('conn-port').value),
                username: document.getElementById('conn-username').value,
                authType: authType
            };

            // 根据认证方式添加相应字段
            if (authType === 'password') {
                connectionDetails.password = document.getElementById('conn-password').value;
            } else {
                connectionDetails.privateKey = document.getElementById('conn-private-key-path').value;
                const passphrase = document.getElementById('conn-passphrase').value;
                if (passphrase) {
                    connectionDetails.passphrase = passphrase;
                }
            }

            if (!window.api || !window.api.ssh) {
                alert('API未正确初始化，请重启应用');
                return;
            }

            const result = await window.api.ssh.connect(connectionDetails);
            if (result.success) {
                // 生成ID并保存会话
                const generatedId = Date.now().toString();
                window.currentSessionId = result.sessionId;

                // 如果不保存密码，则从保存的连接信息中清除密码
                const savedConnectionDetails = {...connectionDetails};
                if (!savePassword) {
                    if (authType === 'password') {
                        savedConnectionDetails.password = '';
                    } else if (authType === 'privateKey' && savedConnectionDetails.passphrase) {
                        savedConnectionDetails.passphrase = '';
                    }
                }

                await window.api.config.saveConnection({
                    ...savedConnectionDetails,
                    id: generatedId,
                    sessionId: result.sessionId
                });

                // 更新状态
                window.uiManager.updateConnectionStatus(true, connectionDetails.name);
                // 更新服务器信息
                window.uiManager.updateServerInfo(true, {
                    name: connectionDetails.name,
                    host: connectionDetails.host
                });

                // 关闭对话框
                document.getElementById('connection-dialog').classList.remove('active');
                document.getElementById('connection-form').reset();

                // 初始化终端 - 先创建空白终端，稍后添加内容
                const terminalInfo = await window.terminalManager.initTerminal(
                    result.sessionId,
                    null,
                    false  // 不显示缓冲区，稍后再加载
                );

                // 保存到会话管理器
                if (terminalInfo) {
                    window.sessionManager.addSession(result.sessionId, generatedId, {
                        term: terminalInfo.term,
                        buffer: '',
                        name: connectionDetails.name
                    });
                }
                
                // 原子激活：activate 返回激活瞬间的缓冲快照并开始 emit，避免
                // fetch→activate 间隙丢掉慢 MOTD 后到达的首个 PS1 提示符
                try {
                    const activateResult = await window.api.ssh.activateSession(result.sessionId);
                    if (activateResult?.success && activateResult.buffer && terminalInfo?.term) {
                        terminalInfo.term.write(activateResult.buffer);
                    }
                } catch (err) {
                    console.warn(`[表单连接] 激活会话失败:`, err);
                }

                // 更新连接列表（已包含活跃状态更新）
                await this.loadConnections();

                // 重置文件管理器状态
                window.fileManager.fileManagerInitialized = false;

                // 保持当前激活的标签类型
                const activeTab = document.querySelector('.tab.active');
                if (activeTab) {
                    const tabId = activeTab.getAttribute('data-tab');
                    
                    // 如果当前活动标签是文件管理器，初始化它
                    if (tabId === 'file-manager') {
                        // 显示加载状态
                        window.uiManager.showFileManagerLoading(true);
                        
                        // 短暂延迟确保会话准备就绪
                        setTimeout(() => {
                            window.fileManager.initFileManager(result.sessionId);
                            window.fileManager.fileManagerInitialized = true;
                        }, 100);
                    }
                    
                    // 触发标签点击以确保UI状态一致
                    activeTab.click();
                }
            } else {
                alert(`连接失败: ${result.error}`);
            }
        } catch (error) {
            console.error('连接错误:', error);
            alert(`连接错误: ${error.message}`);
        } finally {
            this.isConnecting = false;
            window.uiManager.removeLoadingOverlay();
        }
    }
    
    // 处理编辑连接
    async handleEditConnection(editingId) {
        try {
            const authType = document.getElementById('auth-type').value;
            const savePassword = document.getElementById('conn-save-password').checked;

            const connectionDetails = {
                id: editingId, // 保持原有ID
                name: document.getElementById('conn-name').value,
                host: document.getElementById('conn-host').value,
                port: parseInt(document.getElementById('conn-port').value),
                username: document.getElementById('conn-username').value,
                authType: authType
            };

            // 根据认证方式添加相应字段
            if (authType === 'password') {
                if (savePassword) {
                    connectionDetails.password = document.getElementById('conn-password').value;
                }
            } else {
                connectionDetails.privateKey = document.getElementById('conn-private-key-path').value;
                if (savePassword) {
                    const passphrase = document.getElementById('conn-passphrase').value;
                    if (passphrase) {
                        connectionDetails.passphrase = passphrase;
                    }
                }
            }

            // 保存更新的连接
            if (window.api && window.api.config) {
                const result = await window.api.config.saveConnection(connectionDetails);
                if (result) {
                    // 关闭对话框
                    document.getElementById('connection-dialog').classList.remove('active');
                    document.getElementById('connection-form').reset();
                    
                    // 清除编辑标记
                    const form = document.getElementById('connection-form');
                    delete form.dataset.editingId;
                    
                    // 重置提交按钮文本
                    const submitBtn = document.getElementById('connection-submit-btn');
                    if (submitBtn) {
                        submitBtn.innerHTML = `${window.Icons.svg('arrow-right', 16)}连接`;
                    }
                    
                    // 重新加载连接列表
                    await this.loadConnections();
                } else {
                    alert('保存连接失败');
                }
            }
        } catch (error) {
            console.error('编辑连接失败:', error);
            alert(`编辑连接失败: ${error.message}`);
        }
    }
    
    // 存储当前的数据处理监听器移除函数
    currentDataHandlerRemover = null;
    currentClosedHandlerRemover = null;
    
    // 设置SSH数据处理和连接关闭处理
    setupSSHHandlers() {
        this.setupSSHDataHandler();
        this.setupSSHClosedHandler();
    }
    
    // 设置SSH数据处理
    setupSSHDataHandler() {
        if (!window.api || !window.api.ssh) {
            console.error('API未初始化，无法设置SSH数据处理');
            return;
        }

        // 先移除旧的事件监听器
        if (this.currentDataHandlerRemover) {
            this.currentDataHandlerRemover();
            this.currentDataHandlerRemover = null;
        }

        // 添加新的事件监听器：始终按 sessionId 路由到对应的 xterm，
        // 这样后台 session 也能实时收到数据，切回时无需重放缓冲区。
        this.currentDataHandlerRemover = window.api.ssh.onData((_event, data) => {
            const dataStr = data.data;
            const sessionId = data.sessionId;

            // 不再做前端二次缓冲：数据直接 write 进 xterm（自带 scrollback），
            // 首屏回放走后端 ssh_get_session_buffer。原 addToBuffer 既是 100KB 冗余拷贝，
            // 又在每个数据块 console.log，高吞吐输出时拖垮渲染进程。
            const term = window.terminalManager.getTerminalForSession(sessionId);
            if (term) {
                try {
                    term.write(dataStr);
                } catch (error) {
                    console.error(`[setupSSHDataHandler] 写入终端失败:`, error);
                }
            }
        });
    }
    
    // 设置SSH关闭处理
    setupSSHClosedHandler() {
        if (!window.api || !window.api.ssh || !window.api.ssh.onClosed) {
            console.error('API未初始化，无法设置SSH关闭处理');
            return;
        }

        // 先移除旧的事件监听器
        if (this.currentClosedHandlerRemover) {
            this.currentClosedHandlerRemover();
            this.currentClosedHandlerRemover = null;
        }

        // 添加新的事件监听器
        this.currentClosedHandlerRemover = window.api.ssh.onClosed(async (_event, data) => {
            const sessionId = data.sessionId;

            // 移除前记录所属连接，供远程面板"重新连接"按钮使用。
            const connectionId = window.sessionManager.getSession(sessionId)?.connectionId || null;

            // server EOF / exit 命令：彻底移除会话记录。否则 loadConnections 仍能
            // getSessionByConnectionId 命中 → 左侧连接项一直显示 online（绿点 + 断开按钮）。
            window.sessionManager.setSessionActive(sessionId, false);
            window.sessionManager.removeSession(sessionId);

            // 只销毁这个 session 对应的 xterm 实例（不影响其他后台 session）
            window.terminalManager.disposeTerminalInstance(sessionId);

            // 如果是当前活跃会话，清理 UI
            if (sessionId === window.currentSessionId) {
                window.currentSessionId = null;

                const placeholder = document.getElementById('terminal-placeholder');
                if (placeholder) placeholder.classList.remove('hidden');

                window.uiManager.updateConnectionStatus(false);
                window.uiManager.updateServerInfo(false);

                window.fileManager.clearFileManagerCache();
                window.fileManager.fileManagerInitialized = false;

                // 不跳回终端：远程面板提示重新连接，本地面板仍可浏览。
                window.fileManager.renderRemoteEmptyState('连接已断开，请重新连接到服务器', connectionId);
            }

            // 更新连接列表
            await this.loadConnections();
        });
    }
}

// 导出单例实例
const connectionManager = new ConnectionManager();
export default connectionManager;
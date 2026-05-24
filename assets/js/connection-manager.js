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
                        console.log(`双击连接: ${connection.id}, 名称: ${connection.name}`);
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

            // 调试输出
            console.log('加载了', connections.length, '个连接');
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
        console.log(`[switchToSession] 开始切换到连接ID: ${connectionId} 的会话`);

        // 获取会话信息
        const sessionInfo = window.sessionManager.getSessionByConnectionId(connectionId);
        
        // 如果是当前会话，直接返回
        if (sessionInfo && window.currentSessionId === sessionInfo.sessionId) {
            console.log(`[switchToSession] 已经在使用这个会话，无需切换`);
            return true;
        }

        // 清除文件管理器缓存
        window.fileManager.clearFileManagerCache();

        // 保存当前终端状态而不是清空
        const terminalContainer = document.getElementById('terminal-container');
        if (terminalContainer && window.currentSessionId) {
            const currentTerminal = terminalContainer.firstChild;
            if (currentTerminal) {
                currentTerminal.style.display = 'none';
                currentTerminal.dataset.sessionId = window.currentSessionId;
            }
        }
        
        // 创建加载指示器
        const loadingOverlay = document.createElement('div');
        loadingOverlay.className = 'loading-overlay';
        loadingOverlay.innerHTML = '<div class="spinner"></div><div class="loading-text">正在切换会话...</div>';

        const terminalContent = document.querySelector('.terminal-content');
        if (terminalContent) {
            terminalContent.appendChild(loadingOverlay);
        }

        try {
            // 获取会话信息
            const sessionInfo = window.sessionManager.getSessionByConnectionId(connectionId);
            if (!sessionInfo) {
                console.error(`[switchToSession] 找不到连接ID: ${connectionId} 的会话`);
                return false;
            }

            // 如果是当前会话，直接返回
            if (window.currentSessionId === sessionInfo.sessionId) {
                console.log(`[switchToSession] 已经在使用这个会话，无需切换`);
                return true;
            }

            // 检查会话是否有效
            const session = sessionInfo.session;
            if (!session || !session.stream) {
                console.log(`[switchToSession] 会话 ${sessionInfo.sessionId} 无效或已断开连接，尝试重新连接`);

                // 清理旧会话
                if (sessionInfo.sessionId) {
                    window.sessionManager.removeSession(sessionInfo.sessionId);
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

                        // 创建新终端, 先清空容器（不在 initTerminal 内拉 buffer）
                        const terminalInfo = await window.terminalManager.initTerminal(result.sessionId, null, false, true);
                        // 保存到会话管理器
                        window.sessionManager.addSession(result.sessionId, connectionId, {
                            term: terminalInfo.term,
                            buffer: '',
                            name: connection.name
                        });

                        // 同步：fetch buffer → write → activate，保证 active:false 协议下数据不丢
                        try {
                            const bufferResult = await window.api.ssh.getSessionBuffer(result.sessionId);
                            if (bufferResult?.success && bufferResult.buffer && terminalInfo?.term) {
                                terminalInfo.term.write(bufferResult.buffer);
                            }
                        } catch (err) {
                            console.warn(`[switchToSession reconnect] buffer 加载失败:`, err);
                        }
                        try {
                            await window.api.ssh.activateSession(result.sessionId);
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

            // 保存当前会话状态 - 使用异步操作处理
            if (window.currentSessionId && window.terminalManager.activeTerminal) {
                // 标记当前会话为非活跃
                window.sessionManager.setSessionActive(window.currentSessionId, false);

                // 清理终端事件监听器
                try {
                    if (window.terminalManager.currentTerminalDataHandlerDisposer && 
                        typeof window.terminalManager.currentTerminalDataHandlerDisposer === 'function') {
                        window.terminalManager.currentTerminalDataHandlerDisposer();
                        window.terminalManager.currentTerminalDataHandlerDisposer = null;
                        window.terminalManager.currentTerminalDataHandler = null;
                    }
                } catch (err) {
                    console.warn(`[switchToSession] 移除终端数据处理监听器出错:`, err);
                }
            }

            // 先更新全局会话ID和活跃状态，然后再进行其他操作
            window.currentSessionId = sessionInfo.sessionId;
            window.sessionManager.setSessionActive(sessionInfo.sessionId, true);

            // 设置SSH数据和关闭处理
            this.setupSSHHandlers();

            // 先初始化空终端（不在 initTerminal 内拉 buffer）
            const terminalResult = await window.terminalManager.initTerminal(
                sessionInfo.sessionId,
                sessionInfo.session,
                false,
                true
            );

            if (!terminalResult) {
                throw new Error('终端初始化失败');
            }
            window.terminalManager.activeTerminal = terminalResult.term;

            // 同步：fetch buffer → write → activate。
            // 顺序必须是先拿 buffer 再 activate，否则 active:false 协议下 ssh-service
            // 会把缓冲区里 bufferReadLen 之后的内容补 emit，和这里写入的 buffer 重复。
            try {
                const bufferResult = await window.api.ssh.getSessionBuffer(sessionInfo.sessionId);
                if (bufferResult?.success && bufferResult.buffer && window.terminalManager.activeTerminal) {
                    window.terminalManager.activeTerminal.write(bufferResult.buffer);
                }
            } catch (err) {
                console.warn(`[switchToSession] 加载缓冲区数据失败:`, err);
            }
            try {
                const activateResult = await window.api.ssh.activateSession(sessionInfo.sessionId);
                if (activateResult?.sessionId && activateResult.sessionId !== sessionInfo.sessionId) {
                    console.log(`[switchToSession] 会话已重新连接，更新会话ID: ${activateResult.sessionId}`);
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
        } finally {
            // 使用 requestAnimationFrame 延迟移除加载遮罩，避免闪烁
            window.requestAnimationFrame(() => {
                setTimeout(() => {
                    if (terminalContent && terminalContent.contains(loadingOverlay)) {
                        terminalContent.removeChild(loadingOverlay);
                    }
                }, 50);  // 添加小延迟使切换更平滑
            });
        }
    }
    
    // 显示编辑连接对话框
    showEditConnectionDialog(connection) {
        // 填充表单字段
        document.getElementById('conn-name').value = connection.name || '';
        document.getElementById('conn-host').value = connection.host || '';
        document.getElementById('conn-port').value = connection.port || 22;
        document.getElementById('conn-username').value = connection.username || '';
        
        // 设置认证类型
        const authTypeSelect = document.getElementById('auth-type');
        authTypeSelect.value = connection.authType || 'password';
        
        // 触发认证类型变更事件，显示正确的字段
        authTypeSelect.dispatchEvent(new Event('change'));
        
        // 根据认证类型填充相应字段
        if (connection.authType === 'password') {
            document.getElementById('conn-password').value = connection.password || '';
        } else if (connection.authType === 'privateKey') {
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

            // 先清空终端容器，避免看到前一个会话的内容（仅在确实要切换时）
            const terminalContainer = document.getElementById('terminal-container');
            if (terminalContainer) {
                terminalContainer.innerHTML = '';
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
                console.log(`尝试切换到现有会话, 连接ID: ${connection.id}`);

                // 确保会话被标记为活跃状态
                if (sessionInfo.session && !sessionInfo.session.active) {
                    sessionInfo.session.active = true;
                    window.sessionManager.updateSession(sessionInfo.sessionId, {active: true});
                }

                // 使用新的切换功能
                const switchResult = await this.switchToSession(connection.id);

                if (switchResult) {
                    console.log('会话切换成功');
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
            console.log(`建立新连接: ${connection.name}`);
            this.isConnecting = true;
            window.uiManager.createLoadingOverlay('正在连接服务器...');

            const result = await window.api.ssh.connect(connection);

            if (result && result.success) {
                window.currentSessionId = result.sessionId;

                await window.api.config.saveConnection({
                    ...connection,
                    sessionId: result.sessionId
                });

                const terminalInfo = await window.terminalManager.initTerminal(
                    result.sessionId,
                    null,
                    false,
                    true
                );

                if (terminalInfo) {
                    window.sessionManager.addSession(result.sessionId, connection.id, {
                        term: terminalInfo.term,
                        buffer: '',
                        name: connection.name
                    });
                }

                try {
                    const bufferResult = await window.api.ssh.getSessionBuffer(result.sessionId);
                    if (bufferResult && bufferResult.success && bufferResult.buffer && terminalInfo?.term) {
                        terminalInfo.term.write(bufferResult.buffer);
                    }
                } catch (err) {
                    console.warn(`[连接] 加载缓冲区数据失败:`, err);
                }

                try {
                    await window.api.ssh.activateSession(result.sessionId);
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

            // 先清空终端容器，避免看到前一个会话的内容
            const terminalContainer = document.getElementById('terminal-container');
            if (terminalContainer) {
                terminalContainer.innerHTML = '';
            }

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

            console.log('尝试连接...');

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

                const savedConnection = await window.api.config.saveConnection({
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
                    false,  // 不显示缓冲区，稍后再加载
                    true    // 先清空容器
                );

                // 保存到会话管理器
                if (terminalInfo) {
                    window.sessionManager.addSession(result.sessionId, generatedId, {
                        term: terminalInfo.term,
                        buffer: '',
                        name: connectionDetails.name
                    });
                }
                
                // 同步：fetch buffer → write → activate，与 connectToSaved 保持一致
                try {
                    const bufferResult = await window.api.ssh.getSessionBuffer(result.sessionId);
                    if (bufferResult?.success && bufferResult.buffer && terminalInfo?.term) {
                        terminalInfo.term.write(bufferResult.buffer);
                    }
                } catch (err) {
                    console.warn(`[表单连接] 加载缓冲区数据失败:`, err);
                }
                try {
                    await window.api.ssh.activateSession(result.sessionId);
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
                    
                    console.log('连接更新成功');
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
            console.log('已移除旧的SSH数据处理监听器');
        }

        // 添加新的事件监听器
        this.currentDataHandlerRemover = window.api.ssh.onData((event, data) => {
            const dataStr = data.data;
            const sessionId = data.sessionId;

            // 向缓冲区添加数据
            window.sessionManager.addToBuffer(sessionId, dataStr);

            // 如果是当前会话，更新终端显示
            if (sessionId === window.currentSessionId && window.terminalManager.activeTerminal) {
                try {
                    window.terminalManager.activeTerminal.write(dataStr);
                    console.log(`[setupSSHDataHandler] 写入数据到终端，会话ID: ${sessionId}, 数据长度: ${dataStr.length}`);
                } catch (error) {
                    console.error(`[setupSSHDataHandler] 写入数据到终端失败:`, error);
                }
            } else {
                console.log(`[setupSSHDataHandler] 数据已添加到缓冲区，会话ID: ${sessionId}, 数据长度: ${dataStr.length}`);
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
            console.log('已移除旧的SSH关闭处理监听器');
        }

        // 添加新的事件监听器
        this.currentClosedHandlerRemover = window.api.ssh.onClosed(async (event, data) => {
            const sessionId = data.sessionId;

            console.log(`SSH连接关闭: ${sessionId}`);

            // 标记为非活跃
            window.sessionManager.setSessionActive(sessionId, false);

            // 如果是当前活跃会话，清理终端显示
            if (sessionId === window.currentSessionId) {
                window.terminalManager.activeTerminal = null;
                window.currentSessionId = null;
                window.terminalFitAddon = null;

                const terminalContainer = document.getElementById('terminal-container');
                if (terminalContainer) {
                    terminalContainer.innerHTML = '';
                }

                const placeholder = document.getElementById('terminal-placeholder');
                if (placeholder) {
                    placeholder.classList.remove('hidden');
                }

                window.uiManager.updateConnectionStatus(false);
                window.uiManager.updateServerInfo(false);
                
                // 清理文件管理器状态
                window.fileManager.clearFileManagerCache();
                window.fileManager.fileManagerInitialized = false;
                
                // 清空文件管理器视图
                const remoteFilesTbody = document.querySelector('#remote-files tbody');
                if (remoteFilesTbody) {
                    remoteFilesTbody.innerHTML = '';
                }
            }

            // 更新连接列表
            await this.loadConnections();
        });
    }
}

// 导出单例实例
const connectionManager = new ConnectionManager();
export default connectionManager;
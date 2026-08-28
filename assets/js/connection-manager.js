// connection-manager.js
// 处理连接相关功能

import {
    getUngroupedConnections,
    groupConnections,
    matchesConnection,
    moveConnection,
} from './connection-groups.mjs';
import {
    getCurrentSessionId,
    setConnectionDialogOpen,
    setEditingConnection,
    setCurrentSessionId,
} from './app-state.mjs';

const COLLAPSED_GROUPS_KEY = 'sshl.collapsedConnectionGroups';

function escapeHtml(value) {
    return String(value ?? '').replace(/[&<>"']/g, char => ({
        '&': '&amp;',
        '<': '&lt;',
        '>': '&gt;',
        '"': '&quot;',
        "'": '&#39;',
    }[char]));
}

class ConnectionManager {
    constructor() {
        this.isConnecting = false; // 连接中状态标志
        this.activeConnectionAttempt = null;
        this.connectionCancellationSetup = false;
        this.connectionSearchSetup = false;
        this.connectionGroupSetup = false;
        this.draggedConnectionId = null;
        this.collapsedGroups = new Set();
        try {
            const saved = JSON.parse(localStorage.getItem(COLLAPSED_GROUPS_KEY) || '[]');
            if (Array.isArray(saved)) this.collapsedGroups = new Set(saved);
        } catch {}
    }

    setupConnectionSearch() {
        if (this.connectionSearchSetup) return;
        const search = document.getElementById('connection-search');
        if (!search) return;
        this.connectionSearchSetup = true;
        search.addEventListener('input', () => { void this.loadConnections(); });
    }

    setupConnectionGroupInteractions() {
        if (this.connectionGroupSetup) return;
        const connectionList = document.getElementById('connection-list');
        if (!connectionList) return;
        this.connectionGroupSetup = true;

        connectionList.addEventListener('click', event => {
            const header = event.target.closest('.connection-group-header');
            if (!header) return;
            this.toggleConnectionGroup(header.dataset.group);
        });

        connectionList.addEventListener('pointerdown', event => {
            if (event.button !== 0 || event.target.closest('button')) return;
            const item = event.target.closest('.connection-item');
            if (!item) return;
            this.dragState = {
                connectionId: item.dataset.id,
                pointerId: event.pointerId,
                startX: event.clientX,
                startY: event.clientY,
                active: false,
                targetGroup: null,
                beforeId: null,
                hoverTarget: null,
                item,
            };
        });

        connectionList.addEventListener('pointermove', event => {
            const drag = this.dragState;
            if (!drag || drag.pointerId !== event.pointerId) return;
            const distance = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
            if (!drag.active && distance < 6) return;
            if (!drag.active) {
                drag.active = true;
                drag.item.classList.add('dragging');
                drag.item.style.pointerEvents = 'none';
                connectionList.classList.add('pointer-dragging');
                connectionList.setPointerCapture?.(event.pointerId);
            }
            event.preventDefault();
            this.updateConnectionDropTarget(event);
        });

        const finish = event => this.finishConnectionDrag(event);
        connectionList.addEventListener('pointerup', finish);
        connectionList.addEventListener('pointercancel', finish);
        window.addEventListener('pointerup', finish, true);
        window.addEventListener('pointercancel', finish, true);
    }

    updateConnectionDropTarget(event) {
        const drag = this.dragState;
        const connectionList = document.getElementById('connection-list');
        if (!drag || !connectionList) return;
        const element = document.elementFromPoint(event.clientX, event.clientY);
        const targetItem = element?.closest('.connection-item');
        const targetHeader = element?.closest('.connection-group-header');
        const targetDefault = element?.closest('.connection-default-items');
        drag.hoverTarget?.classList.remove('drag-over');
        drag.hoverTarget = null;
        drag.targetGroup = null;
        drag.beforeId = null;

        if (targetItem && targetItem.dataset.id !== drag.connectionId) {
            const targetGroup = targetItem.closest('.connection-group')?.dataset.group || '';
            const rect = targetItem.getBoundingClientRect();
            const placeBefore = event.clientY < rect.top + rect.height / 2;
            const nextItem = targetItem.nextElementSibling?.closest?.('.connection-item');
            drag.targetGroup = targetGroup;
            drag.beforeId = placeBefore ? targetItem.dataset.id : nextItem?.dataset.id || null;
            targetItem.classList.add('drag-over');
            drag.hoverTarget = targetItem;
            return;
        }

        if (targetHeader) {
            const group = targetHeader.closest('.connection-group');
            drag.targetGroup = targetHeader.dataset.group || '';
            if (group?.classList.contains('collapsed')) {
                group.classList.remove('collapsed');
            }
            targetHeader.classList.add('drag-over');
            group?.classList.add('drag-over');
            drag.hoverTarget = targetHeader;
            return;
        }

        if (targetDefault) {
            drag.targetGroup = '';
            targetDefault.classList.add('drag-over');
            drag.hoverTarget = targetDefault;
        }
    }

    finishConnectionDrag(event) {
        const drag = this.dragState;
        if (!drag || drag.pointerId !== event.pointerId) return;
        this.dragState = null;
        if (!drag.active) return;
        event.preventDefault();
        const connectionList = document.getElementById('connection-list');
        drag.item.classList.remove('dragging');
        drag.item.style.pointerEvents = '';
        connectionList?.classList.remove('pointer-dragging');
        connectionList?.querySelectorAll('.drag-over').forEach(element => element.classList.remove('drag-over'));
        connectionList?.releasePointerCapture?.(event.pointerId);
        if (event.type === 'pointercancel' || drag.targetGroup === null) return;

        this.connections = moveConnection(
            this.connections,
            this.groupOrder,
            drag.connectionId,
            drag.targetGroup,
            drag.beforeId,
        );
        this.renderConnectionList();
        void this.persistConnectionLayout(this.connections);
    }

    async persistConnectionLayout(connections = this.connections) {
        const layout = connections.map(connection => ({
            id: connection.id,
            group: connection.group?.trim() || '',
        }));
        if (layout.length) await window.api.config.applyConnectionLayout(layout);
    }

    toggleConnectionGroup(groupName) {
        if (!groupName) return;
        if (this.collapsedGroups.has(groupName)) this.collapsedGroups.delete(groupName);
        else this.collapsedGroups.add(groupName);
        localStorage.setItem(COLLAPSED_GROUPS_KEY, JSON.stringify([...this.collapsedGroups]));
        this.renderConnectionList();
    }


    setupConnectionCancellation() {
        if (this.connectionCancellationSetup) return;
        this.connectionCancellationSetup = true;
        this.setupConnectionSearch();
        this.setupConnectionGroupInteractions();

        document.addEventListener('keydown', (event) => {
            const attempt = this.activeConnectionAttempt;
            if (event.key !== 'Escape' || !attempt || attempt.cancelled || attempt.resolved) return;

            event.preventDefault();
            event.stopImmediatePropagation();
            void this.cancelConnectionAttempt();
        }, true);
    }

    beginConnectionAttempt() {
        const id = window.crypto?.randomUUID
            ? window.crypto.randomUUID()
            : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
        const attempt = {
            id,
            cancelled: false,
            resolved: false,
            sessionId: null
        };

        this.activeConnectionAttempt = attempt;
        this.isConnecting = true;
        window.uiManager.createLoadingOverlay(
            '正在连接服务器...',
            () => { void this.cancelConnectionAttempt(); }
        );
        return attempt;
    }

    async cancelConnectionAttempt() {
        const attempt = this.activeConnectionAttempt;
        if (!attempt || attempt.cancelled || attempt.resolved) return;

        attempt.cancelled = true;
        if (this.activeConnectionAttempt === attempt) {
            this.activeConnectionAttempt = null;
            this.isConnecting = false;
        }
        window.uiManager.removeLoadingOverlay();

        if (attempt.sessionId) {
            await window.api.ssh.disconnect(attempt.sessionId);
            return;
        }

        if (window.api?.ssh?.cancelConnect) {
            await window.api.ssh.cancelConnect(attempt.id);
        }
    }

    async waitForConnection(details, attempt) {
        const result = await window.api.ssh.connect(details, attempt.id);
        if (result?.success) {
            attempt.sessionId = result.sessionId;
        }
        attempt.resolved = true;

        if (attempt.cancelled) {
            if (attempt.sessionId) {
                await window.api.ssh.disconnect(attempt.sessionId);
            }
            return null;
        }

        if (result?.success) {
            window.uiManager.createLoadingOverlay('正在准备终端...');
        }
        return result;
    }

    finishConnectionAttempt(attempt) {
        if (this.activeConnectionAttempt !== attempt) return;

        this.activeConnectionAttempt = null;
        this.isConnecting = false;
        window.uiManager.removeLoadingOverlay();
    }

    async isSessionAlive(sessionId) {
        if (!window.api?.ssh?.validateSession) return true;

        const result = await window.api.ssh.validateSession(sessionId);
        return Boolean(result?.success && result.alive);
    }

    discardStaleSession(sessionInfo) {
        if (!sessionInfo) return;

        const { sessionId, session } = sessionInfo;
        window.sessionManager.removeSession(sessionId);
        window.terminalManager.disposeTerminalInstance(sessionId);

        if (getCurrentSessionId() === sessionId) {
            setCurrentSessionId(null);

            const placeholder = document.getElementById('terminal-placeholder');
            if (placeholder) placeholder.classList.remove('hidden');

            window.uiManager.updateConnectionStatus(false);
            window.uiManager.updateServerInfo(false);
            window.fileManager?.clearCache?.();
            window.fileManager?.showRemoteEmptyState(
                '连接已失效，正在重新连接...',
                session?.connectionId || null
            );
        }

        const activeConnectionId = getCurrentSessionId()
            ? window.sessionManager.getSession(getCurrentSessionId())?.connectionId || null
            : null;
        this.updateActiveConnectionItem(activeConnectionId);
    }
    
    // 加载连接列表
    async loadConnections() {
        try {
            if (!window.api?.config) {
                console.error('API未初始化，无法加载连接列表');
                return;
            }

            const [connections, groupOrder] = await Promise.all([
                window.api.config.getConnections(),
                window.api.config.getConnectionGroups(),
            ]);
            this.connections = Array.isArray(connections) ? connections : [];
            this.groupOrder = Array.isArray(groupOrder) ? groupOrder : [];
            this.renderConnectionList();
        } catch (error) {
            console.error('加载连接失败:', error);
        }
    }

    renderConnectionList() {
        const connectionList = document.getElementById('connection-list');
        if (!connectionList) return;
        const query = document.getElementById('connection-search')?.value || '';
        const visibleConnections = this.connections.filter(connection => matchesConnection(connection, query));
        const ungroupedConnections = getUngroupedConnections(visibleConnections);
        const groups = groupConnections(visibleConnections, this.groupOrder);

        connectionList.innerHTML = '';
        if (!visibleConnections.length) {
            connectionList.innerHTML = this.connections.length
                ? '<div class="no-connections">没有匹配的连接</div>'
                : '<div class="no-connections">没有保存的连接</div>';
            return;
        }

        const defaultItems = document.createElement('div');
        defaultItems.className = 'connection-default-items';
        for (const connection of ungroupedConnections) defaultItems.appendChild(this.createConnectionItem(connection));
        connectionList.appendChild(defaultItems);

        for (const group of groups) {
            const groupElement = document.createElement('section');
            groupElement.className = 'connection-group';
            groupElement.dataset.group = group.name;
            const collapsed = this.collapsedGroups.has(group.name);
            if (collapsed) groupElement.classList.add('collapsed');

            const header = document.createElement('button');
            header.type = 'button';
            header.className = 'connection-group-header';
            header.dataset.group = group.name;
            header.setAttribute('aria-expanded', String(!collapsed));
            header.innerHTML = `${window.Icons.svg(collapsed ? 'chevron-right' : 'chevron-down', 14, 2.25)}<span class="connection-group-name"></span><span class="connection-group-count">${group.connections.length}</span>`;
            header.querySelector('.connection-group-name').textContent = group.name;
            groupElement.appendChild(header);

            const items = document.createElement('div');
            items.className = 'connection-group-items';
            for (const connection of group.connections) items.appendChild(this.createConnectionItem(connection));
            groupElement.appendChild(items);
            connectionList.appendChild(groupElement);
        }
    }

    createConnectionItem(connection) {
        const existingSessionInfo = window.sessionManager.getSessionByConnectionId(connection.id);
        const isConnected = existingSessionInfo !== null;
        const isActive = isConnected && existingSessionInfo.sessionId === getCurrentSessionId();
        const statusClass = isConnected ? 'online' : 'offline';
        const item = document.createElement('div');
        item.className = 'connection-item';
        item.dataset.id = connection.id;
        item.dataset.active = isActive ? 'true' : 'false';
        item.dataset.connected = isConnected ? 'true' : 'false';
        item.dataset.name = connection.name;
        const disconnectBtn = isConnected ? `<button class="icon-button disconnect-connection" data-session-id="${escapeHtml(existingSessionInfo.sessionId)}" title="断开连接">${window.Icons.svg('power', 14, 2.5)}</button>` : '';
        item.innerHTML = `
            <div class="connection-status-indicator ${statusClass}"></div>
            <div class="connection-name">${escapeHtml(connection.name)}</div>
            <div class="connection-actions">
                ${disconnectBtn}
                <button class="icon-button edit-connection" data-id="${escapeHtml(connection.id)}" title="${isConnected ? '断开后才能编辑' : '编辑连接'}"${isConnected ? ' disabled' : ''}>${window.Icons.svg('square-pen', 14, 2.5)}</button>
                <button class="icon-button delete-connection" data-id="${escapeHtml(connection.id)}" title="${isConnected ? '断开后才能删除' : '删除连接'}"${isConnected ? ' disabled' : ''}>${window.Icons.svg('trash-2', 14, 2.5)}</button>
            </div>
        `;
        item.addEventListener('dblclick', async () => { await this.connectToSaved(connection.id); });
        item.addEventListener('mouseenter', window.uiManager.handleItemHover);
        item.addEventListener('mouseleave', window.uiManager.handleItemLeave);
        return item;
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
        let sessionInfo = window.sessionManager.getSessionByConnectionId(connectionId);
        if (!sessionInfo) {
            return false;
        }

        if (!await this.isSessionAlive(sessionInfo.sessionId)) {
            this.discardStaleSession(sessionInfo);
            return false;
        }

        const activationResult = await window.api.ssh.activateSession(sessionInfo.sessionId);
        if (!activationResult?.success) {
            this.discardStaleSession(sessionInfo);
            return false;
        }

        if (getCurrentSessionId() === sessionInfo.sessionId) {
            return true;
        }

        try {
            window.fileManager?.clearCache?.();

            sessionInfo = window.sessionManager.getSessionByConnectionId(connectionId);
            if (!sessionInfo) {
                console.error(`[switchToSession] 找不到连接ID: ${connectionId} 的会话`);
                return false;
            }

            // 检查会话是否有效。Tauri 后端独立管理 session 生命周期，前端没有 stream 字段；
            // 后端存活探测已在切换前完成，这里只处理前端会话对象缺失的兼容路径。
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
                        setCurrentSessionId(result.sessionId);

                        // 创建该 session 的 xterm 实例（其他后台 session 的 host 会被隐藏，但不销毁）
                        const terminalInfo = await window.terminalManager.initTerminal(result.sessionId, null, false);
                        // 保存到会话管理器
                        window.sessionManager.addSession(result.sessionId, connectionId, {
                            term: terminalInfo.term,
                            buffer: '',
                            name: connection.name
                        });

                        // 原子激活：缓冲快照由后端作为首个 ssh:data 事件发出，
                        // 与后续数据同通道保序（invoke 返回快照会与事件乱序）
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
            if (getCurrentSessionId() && getCurrentSessionId() !== sessionInfo.sessionId) {
                window.sessionManager.setSessionActive(getCurrentSessionId(), false);
            }

            setCurrentSessionId(sessionInfo.sessionId);
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
        setEditingConnection(connection);
        setConnectionDialogOpen(true);
    }
    
    // 连接到保存的连接
    async connectToSaved(id) {
        // 如果已经在连接中，则忽略
        if (this.isConnecting) return;

        let attempt = null;
        try {
            if (!window.api) {
                alert('API未初始化，请重启应用');
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
            attempt = this.beginConnectionAttempt();

            const result = await this.waitForConnection(connection, attempt);
            if (!result) return;

            if (result && result.success) {
                setCurrentSessionId(result.sessionId);

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

                // 原子激活：缓冲快照由后端作为首个 ssh:data 事件发出，
                // 与后续数据同通道保序（invoke 返回快照会与事件乱序）
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

            } else {
                alert(`连接失败: ${result ? result.error || 'unknown error' : 'unknown error'}`);
            }
        } catch (error) {
            if (attempt?.cancelled) return;
            console.error('连接错误:', error);
            alert(`连接错误: ${error ? error.message || '未知错误' : '未知错误'}`);
        } finally {
            if (attempt) {
                this.finishConnectionAttempt(attempt);
            }
        }
    }
    
    // 提交新连接并建立会话
    async submitConnection(details) {
        // 如果已经在连接中，则忽略
        if (this.isConnecting) return;

        let attempt = null;
        try {
            const authType = details.authType;
            const savePassword = Boolean(details.savePassword);
            const connectionDetails = {
                name: details.name,
                host: details.host,
                port: parseInt(details.port),
                username: details.username,
                group: (details.group || '').trim(),
                authType: authType
            };

            // 根据认证方式添加相应字段
            if (authType === 'password') {
                connectionDetails.password = details.password;
            } else {
                connectionDetails.privateKey = details.privateKey;
                const passphrase = details.passphrase;
                if (passphrase) {
                    connectionDetails.passphrase = passphrase;
                }
            }

            if (!window.api || !window.api.ssh) {
                alert('API未正确初始化，请重启应用');
                return;
            }

            attempt = this.beginConnectionAttempt();
            const result = await this.waitForConnection(connectionDetails, attempt);
            if (!result) return;

            if (result.success) {
                // 生成ID并保存会话
                const generatedId = Date.now().toString();
                setCurrentSessionId(result.sessionId);

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
                setConnectionDialogOpen(false);
                setEditingConnection(null);

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
                
                // 原子激活：缓冲快照由后端作为首个 ssh:data 事件发出，
                // 与后续数据同通道保序（invoke 返回快照会与事件乱序）
                try {
                    await window.api.ssh.activateSession(result.sessionId);
                } catch (err) {
                    console.warn(`[连接] 激活会话失败:`, err);
                }

                // 更新连接列表
                await this.loadConnections();

            } else {
                alert(`连接失败: ${result.error}`);
            }
        } catch (error) {
            if (attempt?.cancelled) return;
            console.error('连接错误:', error);
            alert(`连接错误: ${error.message}`);
        } finally {
            if (attempt) {
                this.finishConnectionAttempt(attempt);
            }
        }
    }
    
    // 保存编辑后的连接
    async saveEditedConnection(id, details) {
        try {
            const authType = details.authType;
            const savePassword = Boolean(details.savePassword);
            const connectionDetails = {
                id: id, // 保持原有ID
                name: details.name,
                host: details.host,
                port: parseInt(details.port),
                username: details.username,
                group: (details.group || '').trim(),
                authType: authType
            };

            // 根据认证方式添加相应字段
            if (authType === 'password') {
                if (savePassword) {
                    connectionDetails.password = details.password;
                }
            } else {
                connectionDetails.privateKey = details.privateKey;
                if (savePassword) {
                    const passphrase = details.passphrase;
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
                    setConnectionDialogOpen(false);
                    setEditingConnection(null);
                    
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
            if (sessionId === getCurrentSessionId()) {
                setCurrentSessionId(null);

                const placeholder = document.getElementById('terminal-placeholder');
                if (placeholder) placeholder.classList.remove('hidden');

                window.uiManager.updateConnectionStatus(false);
                window.uiManager.updateServerInfo(false);

                window.fileManager?.clearCache?.();

                // 不跳回终端：远程面板提示重新连接，本地面板仍可浏览。
                window.fileManager?.showRemoteEmptyState('连接已断开，请重新连接到服务器', connectionId);
            }

            // 更新连接列表
            await this.loadConnections();
        });
    }
}

// 导出单例实例
const connectionManager = new ConnectionManager();
export default connectionManager;

// terminal-manager.js
// 专门处理终端相关功能

// 防抖函数
function debounce(func, wait) {
    let timeout;
    return function executedFunction(...args) {
        const later = () => {
            clearTimeout(timeout);
            func(...args);
        };
        clearTimeout(timeout);
        timeout = setTimeout(later, wait);
    };
}

class TerminalManager {
    constructor() {
        this.activeTerminal = null;
        this.terminalFitAddon = null;
        this.currentTerminalDataHandler = null;
        this.currentTerminalDataHandlerDisposer = null;
        this.isTabSwitching = false;
        // 每个 sessionId 维护独立的 xterm 实例：{ term, fitAddon, host, dataDisposer, cleanup }
        // 切换会话只是 toggle display，不销毁实例 —— 保留 scrollback 与 TUI 状态。
        this.terminals = new Map();
        this.activeSessionId = null;
        
        // 防抖的resize函数
        this.resizeTerminal = debounce(() => {
            if (this.terminalFitAddon && this.activeTerminal && window.currentSessionId) {
                try {
                    this.terminalFitAddon.fit();
                    
                    const dimensions = this.terminalFitAddon.proposeDimensions();
                    if (dimensions && dimensions.cols && dimensions.rows) {
                        window.api.ssh.resize(window.currentSessionId, dimensions.cols, dimensions.rows)
                            .catch(err => {
                                // 只记录非会话相关的错误，避免切换会话时的噪音
                                if (!err.message || !err.message.includes('会话未找到')) {
                                    console.error('调整终端大小失败:', err);
                                }
                            });
                    }
                } catch (err) {
                    console.error('调整终端大小失败:', err);
                }
            }
        }, 100);
    }
    
    // 创建终端实例。container 可传 DOM 元素或元素 id。
    async createXterm(container, options = {}) {
        if (typeof container === 'string') {
            container = document.getElementById(container);
        }
    
        if (!window.Terminal || !window.FitAddon) {
            console.log('Loading Terminal and FitAddon scripts dynamically');
            // 动态加载脚本
            return new Promise((resolve, reject) => {
                // 先加载xterm.js
                const xtermScript = document.createElement('script');
                xtermScript.src = 'app://node_modules/xterm/lib/xterm.js';
    
                // 加载样式
                const xtermStylesheet = document.createElement('link');
                xtermStylesheet.rel = 'stylesheet';
                xtermStylesheet.href = 'app://node_modules/xterm/css/xterm.css';
                document.head.appendChild(xtermStylesheet);
    
                xtermScript.onload = () => {
                    // 加载xterm.js后，加载fit addon
                    const fitScript = document.createElement('script');
                    fitScript.src = 'app://node_modules/xterm-addon-fit/lib/xterm-addon-fit.js';
    
                    fitScript.onload = () => {
                        try {
                            // 创建终端实例
                            const term = new Terminal({
                                cursorBlink: true,
                                cursorStyle: 'bar',
                                fontSize: 14,
                                fontFamily: 'monospace',
                                theme: {
                                    background: '#1e1e1e',
                                    foreground: '#f0f0f0',
                                    cursor: '#ffffff'
                                },
                                allowTransparency: false,
                                rendererType: 'dom',
                                ...options
                            });
    
                            // 创建fit addon
                            const fitAddon = new FitAddon.FitAddon();
                            term.loadAddon(fitAddon);
    
                            term.open(container);
                            fitAddon.fit();
    
                            // 添加窗口大小调整事件监听器（使用防抖）
                            const resizeHandler = debounce(() => {
                                if (fitAddon && term) {
                                    fitAddon.fit();
                                }
                            }, 50);
                            
                            window.addEventListener('resize', resizeHandler);
                            
                            // 存储清理函数
                            term._resizeHandler = resizeHandler;
                            term._cleanup = () => {
                                window.removeEventListener('resize', resizeHandler);
                            };
    
                            // 强制延迟以确保适当的大小
                            setTimeout(() => {
                                fitAddon.fit();
                            }, 100);
    
                            resolve({term, fitAddon});
                        } catch (error) {
                            console.error('创建终端错误:', error);
                            reject(error);
                        }
                    };
    
                    fitScript.onerror = (error) => {
                        console.error('加载 FitAddon 失败:', error);
                        reject(new Error('Failed to load xterm-addon-fit.js'));
                    };
    
                    document.head.appendChild(fitScript);
                };
    
                xtermScript.onerror = (error) => {
                    console.error('加载 xterm.js 失败:', error);
                    reject(new Error('Failed to load xterm.js'));
                };
    
                document.head.appendChild(xtermScript);
            });
        } else {
            // 脚本已加载，直接创建终端
            return new Promise((resolve, reject) => {
                try {
                    // 创建终端实例
                    const term = new Terminal({
                        cursorBlink: true,
                        cursorStyle: 'bar',
                        fontSize: 14,
                        fontFamily: 'monospace',
                        theme: {
                            background: '#1e1e1e',
                            foreground: '#f0f0f0',
                            cursor: '#ffffff'
                        },
                        allowTransparency: false,
                        rendererType: 'dom',
                        ...options
                    });
    
                    // 创建fit addon
                    const fitAddon = new FitAddon.FitAddon();
                    term.loadAddon(fitAddon);
    
                    term.open(container);
                    fitAddon.fit();
    
                    // 添加窗口大小调整事件监听器
                    window.addEventListener('resize', () => {
                        fitAddon.fit();
                    });
    
                    // 强制延迟以确保适当的大小
                    setTimeout(() => {
                        fitAddon.fit();
                    }, 100);
    
                    resolve({term, fitAddon});
                } catch (error) {
                    console.error('创建终端错误:', error);
                    reject(error);
                }
            });
        }
    }
    
    // 取得指定 session 的 xterm 实例（用于路由后端 ssh:data 事件）
    getTerminalForSession(sessionId) {
        const e = this.terminals.get(sessionId);
        return e ? e.term : null;
    }

    // 隐藏所有 per-session host 容器
    _hideAllTerminalHosts() {
        for (const entry of this.terminals.values()) {
            if (entry.host) entry.host.style.display = 'none';
        }
    }

    // 把已存在的 session 终端展示出来，并设置为活跃实例
    _showExistingTerminal(sessionId, entry) {
        this._hideAllTerminalHosts();
        entry.host.style.display = 'block';
        this.activeTerminal = entry.term;
        this.terminalFitAddon = entry.fitAddon;
        this.activeSessionId = sessionId;
        window.terminalFitAddon = entry.fitAddon;

        this.createTerminalTab(sessionId);
        const placeholder = document.getElementById('terminal-placeholder');
        if (placeholder) placeholder.classList.add('hidden');

        requestAnimationFrame(() => {
            try { entry.term.focus(); } catch (err) { console.warn('[showTerminal] focus 失败:', err); }
            try {
                entry.fitAddon.fit();
                const d = entry.fitAddon.proposeDimensions();
                if (d && d.cols && d.rows && window.api?.ssh) {
                    window.api.ssh.resize(sessionId, d.cols, d.rows).catch(() => {});
                }
            } catch (err) {
                console.warn('[showTerminal] fit 失败:', err);
            }
        });
    }

    // 初始化终端：相同 sessionId 复用现有 xterm 实例（保留 scrollback），
    // 否则创建新的 host + xterm 并加入 this.terminals。
    // 返回 { term, fitAddon, isNew }
    async initTerminal(sessionId, _existingSession = null, _showBuffer = false) {
        try {
            const container = document.getElementById('terminal-container');
            if (!container) {
                console.error('找不到终端容器');
                return null;
            }

            // 容器一次性初始化为相对定位，便于子 host 绝对填充
            if (!container.dataset.multiHostReady) {
                container.style.position = 'relative';
                container.dataset.multiHostReady = '1';
            }

            const existing = this.terminals.get(sessionId);
            if (existing) {
                console.log(`[initTerminal] 复用 session ${sessionId} 的终端实例`);
                this._showExistingTerminal(sessionId, existing);
                return { term: existing.term, fitAddon: existing.fitAddon, isNew: false };
            }

            console.log(`[initTerminal] 为 session ${sessionId} 创建新终端`);

            // 隐藏其他 session 的 host，但不销毁
            this._hideAllTerminalHosts();

            // 创建专属 host 容器
            const host = document.createElement('div');
            host.className = 'term-host';
            host.dataset.sessionId = sessionId;
            host.style.position = 'absolute';
            host.style.inset = '0';
            container.appendChild(host);

            const termOptions = {
                cursorBlink: true,
                cursorStyle: 'bar',
                fontSize: 14,
                fontFamily: 'monospace',
                theme: {
                    background: '#1e1e1e',
                    foreground: '#FBF74B',
                    cursor: '#FBF74B'
                },
                allowTransparency: true,
                rendererType: 'canvas',
                blinkInterval: 500
            };

            const { term, fitAddon } = await this.createXterm(host, termOptions);

            // 数据处理器绑定到该 session（每个 xterm 各自独立）
            let dataDisposer = null;
            try {
                const disposer = term.onData((data) => {
                    if (window.api?.ssh) {
                        window.api.ssh.sendData(sessionId, data)
                            .catch(err => console.error('发送数据失败:', err));
                    }
                });
                if (typeof disposer === 'function') {
                    dataDisposer = disposer;
                } else if (disposer && typeof disposer.dispose === 'function') {
                    dataDisposer = () => disposer.dispose();
                }
            } catch (err) {
                console.error('[initTerminal] 注册 onData 失败:', err);
            }

            const entry = {
                term,
                fitAddon,
                host,
                dataDisposer,
                cleanup: term._cleanup || null
            };
            this.terminals.set(sessionId, entry);

            this.activeTerminal = term;
            this.terminalFitAddon = fitAddon;
            this.activeSessionId = sessionId;
            window.terminalFitAddon = fitAddon;

            this.createTerminalTab(sessionId);

            const placeholder = document.getElementById('terminal-placeholder');
            if (placeholder) placeholder.classList.add('hidden');

            requestAnimationFrame(() => {
                try { term.focus(); } catch (err) { console.warn('[initTerminal] focus 失败:', err); }
                try {
                    fitAddon.fit();
                    const d = fitAddon.proposeDimensions();
                    if (d && d.cols && d.rows && window.api?.ssh) {
                        window.api.ssh.resize(sessionId, d.cols, d.rows)
                            .catch(err => console.error('初始化调整终端大小失败:', err));
                    }
                } catch (err) {
                    console.warn('[initTerminal] fit 失败:', err);
                }
            });

            return { term, fitAddon, isNew: true };
        } catch (error) {
            console.error('初始化终端失败:', error);
            throw error;
        }
    }

    // 销毁单个 session 的 xterm 实例（断开连接时调用，切换会话不会调用）
    disposeTerminalInstance(sessionId) {
        const entry = this.terminals.get(sessionId);
        if (!entry) return;
        try {
            if (typeof entry.dataDisposer === 'function') entry.dataDisposer();
        } catch (err) { console.warn('[dispose] dataDisposer 失败:', err); }
        try {
            if (typeof entry.cleanup === 'function') entry.cleanup();
        } catch (err) { console.warn('[dispose] cleanup 失败:', err); }
        try { entry.term.dispose(); } catch (err) { console.warn('[dispose] term.dispose 失败:', err); }
        if (entry.host && entry.host.parentNode) entry.host.parentNode.removeChild(entry.host);
        this.terminals.delete(sessionId);
        if (this.activeSessionId === sessionId) {
            this.activeTerminal = null;
            this.terminalFitAddon = null;
            this.activeSessionId = null;
            window.terminalFitAddon = null;
        }
    }
    
    // 创建终端标签
    createTerminalTab(sessionId) {
        const tabsContainer = document.getElementById('terminal-tabs-left');
        if (!tabsContainer) {
            return;
        }
    
        tabsContainer.innerHTML = '';
    
        const tab = document.createElement('div');
        tab.className = 'terminal-tab active';
        tab.innerHTML = `
          <span>终端</span>
          <button class="close-tab" data-session-id="${sessionId}">×</button>
        `;
    
        tabsContainer.appendChild(tab);
    
        // 为关闭按钮添加事件监听
        const closeBtn = tab.querySelector('.close-tab');
        if (closeBtn) {
            closeBtn.addEventListener('click', () => this.disconnectSession(sessionId));
        }
    }

    /**
     * 断开指定会话；如果是当前活跃会话，会清理终端 UI。
     * 切换连接不会触发此方法（切换只换前台显示）。
     */
    async disconnectSession(sessionId) {
        if (!sessionId) return;
        const isActive = window.currentSessionId === sessionId;

        try {
            await window.api.ssh.disconnect(sessionId);
        } catch (error) {
            console.error('断开连接失败:', error);
        }

        // 移除会话记录
        window.sessionManager.removeSession(sessionId);

        // 只销毁这个 session 对应的 xterm 实例，不影响其他后台 session
        this.disposeTerminalInstance(sessionId);

        if (isActive) {
            window.currentSessionId = null;

            const placeholder = document.getElementById('terminal-placeholder');
            if (placeholder) placeholder.classList.remove('hidden');

            const tabsContainer = document.getElementById('terminal-tabs-left');
            if (tabsContainer) tabsContainer.innerHTML = '';

            window.uiManager.updateConnectionStatus(false);
            window.uiManager.updateServerInfo(false);
        }

        await window.connectionManager.loadConnections();
    }
    
    // 确保终端可见
    ensureTerminalVisible() {
        const container = document.getElementById('terminal-container');
        const placeholder = document.getElementById('terminal-placeholder');
    
        if (container) {
            container.style.display = 'block';
        }
    
        if (placeholder) {
            placeholder.classList.add('hidden');
        }
    
        // 只在需要时调整大小（不是在标签切换期间）
        if (this.terminalFitAddon && !this.isTabSwitching) {
            this.resizeTerminal();
        }
    
        // 只在不是标签切换期间聚焦
        if (this.activeTerminal && !this.isTabSwitching) {
            setTimeout(() => {
                try {
                    this.activeTerminal.focus();
                } catch (err) {
                    console.warn('聚焦终端失败:', err);
                }
            }, 100);
        }
    }
    
    // 调整终端大小
    resizeTerminal() {
        if (this.terminalFitAddon && this.activeTerminal && window.currentSessionId) {
            this.terminalFitAddon.fit();
    
            // 获取并发送更新的终端尺寸
            const dimensions = this.terminalFitAddon.proposeDimensions();
            if (dimensions && window.api && window.api.ssh) {
                window.api.ssh.resize(window.currentSessionId, dimensions.cols, dimensions.rows)
                    .catch(err => {
                        // 只记录非会话相关的错误，避免切换会话时的噪音
                        if (!err.message || !err.message.includes('会话未找到')) {
                            console.error('调整终端大小失败:', err);
                        }
                    });
            }
        }
    }
}

// 导出单例实例
const terminalManager = new TerminalManager();
export default terminalManager;
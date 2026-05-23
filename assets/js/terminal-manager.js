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
    
    // 创建终端实例
    async createXterm(containerId, options = {}) {
        const container = document.getElementById(containerId);
    
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
    
    // 初始化终端
    async initTerminal(sessionId, existingSession = null, showBuffer = true, clearContainerFirst = false) {
        try {
            console.log(`[initTerminal] 开始初始化终端 - 会话ID: ${sessionId}`);
            console.log(`[initTerminal] 现有会话信息:`, existingSession ? {
                active: existingSession.active,
                hasStream: !!existingSession.stream,
                bufferLength: existingSession.buffer ? existingSession.buffer.length : 0
            } : '无');
    
            const container = document.getElementById('terminal-container');
            if (!container) {
                console.error('找不到终端容器');
                return null;
            }

            // 如果需要先清理容器，立即清空（解决切换服务器时的显示问题）
            if (clearContainerFirst) {
                container.innerHTML = '';
            }
    
            // 正确销毁现有终端
            if (this.activeTerminal) {
                console.log(`[initTerminal] 正在销毁之前的终端实例`);
                try {
                    // 先移除数据处理程序
                    if (this.currentTerminalDataHandlerDisposer && 
                        typeof this.currentTerminalDataHandlerDisposer === 'function') {
                        this.currentTerminalDataHandlerDisposer(); // 调用dispose函数移除监听器
                        this.currentTerminalDataHandlerDisposer = null;
                        this.currentTerminalDataHandler = null;
                        console.log(`[initTerminal] 已移除终端数据处理程序`);
                    } else if (this.currentTerminalDataHandlerDisposer) {
                        console.warn(`[initTerminal] currentTerminalDataHandlerDisposer 不是函数，无法调用`);
                        this.currentTerminalDataHandlerDisposer = null;
                        this.currentTerminalDataHandler = null;
                    }
    
                    // 清理自定义的事件监听器
                    if (this.activeTerminal._cleanup) {
                        this.activeTerminal._cleanup();
                    }
                    
                    // 然后销毁终端
                    this.activeTerminal.dispose();
                    this.activeTerminal = null;
                    console.log(`[initTerminal] 已销毁旧终端`);
                } catch (err) {
                    console.warn(`[initTerminal] 销毁之前的终端实例出错:`, err);
                }
            }
    
            // 清理容器（如果前面没有清理过）
            if (!clearContainerFirst) {
                container.innerHTML = '';
            }
    
            // 基本终端选项
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
    
            // 创建新终端实例
            const result = await this.createXterm('terminal-container', termOptions);
            const term = result.term;
            const fitAddon = result.fitAddon;
    
            // 确保终端可见并聚焦
            container.style.display = 'block';
            setTimeout(() => {
                if (term) {
                    try {
                        term.focus();
                    } catch (err) {
                        console.warn(`[initTerminal] 无法聚焦终端:`, err);
                    }
                }
            }, 50);
    
            // 获取会话缓冲区数据
            let sessionBuffer = '';
            try {
                // 从服务获取最新的会话缓冲区
                if (window.api && window.api.ssh && window.api.ssh.getSessionBuffer) {
                    const updatedSessionInfo = await window.api.ssh.getSessionBuffer(sessionId);
                    if (updatedSessionInfo && updatedSessionInfo.success) {
                        sessionBuffer = updatedSessionInfo.buffer || '';
                        console.log(`[initTerminal] 成功从服务获取缓冲区，长度: ${sessionBuffer.length}`);
                    }
                }
            } catch (err) {
                console.warn(`[initTerminal] 获取会话缓冲区失败:`, err);
                // 使用本地缓存的缓冲区作为后备
                if (existingSession && existingSession.buffer) {
                    sessionBuffer = existingSession.buffer;
                    console.log(`[initTerminal] 使用本地缓存的缓冲区，长度: ${sessionBuffer.length}`);
                }
            }
    
            // 恢复缓冲区数据
            if (showBuffer && sessionBuffer) {
                console.log(`[initTerminal] 恢复会话 ${sessionId} 的终端缓冲区`);
                term.write(sessionBuffer);
            } else if (showBuffer && existingSession && existingSession.buffer) {
                console.log(`[initTerminal] 恢复会话 ${sessionId} 的终端缓冲区（使用现有会话）`);
                term.write(existingSession.buffer);
            } else {
                console.log(`[initTerminal] 不显示会话 ${sessionId} 的缓冲区数据`);
                // 不写入缓冲区数据
            }
    
            // 设置全局变量
            this.activeTerminal = term;
            this.terminalFitAddon = fitAddon;
            window.terminalFitAddon = fitAddon; // 保持兼容，以后应该删除这种全局变量
    
            // 设置新的终端数据处理
            this.currentTerminalDataHandler = (data) => {
                if (window.api && window.api.ssh && window.currentSessionId) {
                    console.log(`[terminal data] 发送数据到会话 ${window.currentSessionId}, 数据长度: ${data.length}`);
                    window.api.ssh.sendData(window.currentSessionId, data)
                        .catch(err => console.error('发送数据失败:', err));
                }
            };
    
            // 保存dispose函数以便后续移除监听器
            try {
                const disposer = term.onData(this.currentTerminalDataHandler);
                // 确保返回的是一个函数
                if (typeof disposer === 'function') {
                    this.currentTerminalDataHandlerDisposer = disposer;
                    console.log(`[initTerminal] 成功注册终端数据处理程序`);
                } else {
                    console.warn(`[initTerminal] term.onData 返回的不是函数: ${typeof disposer}`);
                    // 创建一个空函数作为替代
                    this.currentTerminalDataHandlerDisposer = () => {
                        console.log('[initTerminal] 使用替代的dispose函数');
                        // 尝试使用其他方式移除监听器
                        if (term && term._events && term._events.data) {
                            // 如果可能，直接清除事件监听器
                            term._events.data = null;
                        }
                    };
                }
            } catch (err) {
                console.error(`[initTerminal] 注册终端数据处理程序出错:`, err);
                // 创建一个空函数作为替代
                this.currentTerminalDataHandlerDisposer = () => {};
            }
    
            // 创建标签
            this.createTerminalTab(sessionId);
    
            // 隐藏占位符
            const placeholder = document.getElementById('terminal-placeholder');
            if (placeholder) {
                placeholder.classList.add('hidden');
            }
    
            // 确保终端可见并聚焦
            container.style.display = 'block';
            setTimeout(() => {
                if (term) {
                    try {
                        term.focus();
                    } catch (err) {
                        console.warn(`[initTerminal] 无法聚焦终端:`, err);
                    }
                }
            }, 50);
    
            // 调整终端大小并发送尺寸信息
            setTimeout(() => {
                if (fitAddon) {
                    try {
                        fitAddon.fit();
    
                        // 获取并发送终端尺寸
                        const dimensions = fitAddon.proposeDimensions();
                        if (dimensions && window.api && window.api.ssh) {
                            window.api.ssh.resize(sessionId, dimensions.cols, dimensions.rows)
                                .catch(err => console.error('初始化调整终端大小失败:', err));
                        }
                    } catch (err) {
                        console.warn(`[initTerminal] 调整终端大小出错:`, err);
                    }
                }
            }, 100);
    
            return {term, fitAddon};
        } catch (error) {
            console.error('初始化终端失败:', error);
            throw error;
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

        // 始终移除会话记录
        window.sessionManager.removeSession(sessionId);

        if (isActive) {
            this.activeTerminal = null;
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

            const tabsContainer = document.getElementById('terminal-tabs-left');
            if (tabsContainer) {
                tabsContainer.innerHTML = '';
            }

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
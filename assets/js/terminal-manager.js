// terminal-manager.js
// 专门处理终端相关功能

import { getTerminalSettings } from './settings.js';

// Palette index → "#rrggbb"（xterm 256 色标准调色板）
const ANSI_BASE_16 = [
    '#000000','#cd3131','#0dbc79','#e5e510','#2472c8','#bc3fbc','#11a8cd','#e5e5e5',
    '#666666','#f14c4c','#23d18b','#f5f543','#3b8eea','#d670d6','#29b8db','#e5e5e5'
];
function paletteToHex(idx) {
    if (idx < 16) return ANSI_BASE_16[idx];
    if (idx >= 232) {
        const v = 8 + (idx - 232) * 10;
        const h = Math.min(255, v).toString(16).padStart(2, '0');
        return '#' + h + h + h;
    }
    const n = idx - 16;
    const r = Math.floor(n / 36), g = Math.floor((n % 36) / 6), b = n % 6;
    const lvl = v => (v === 0 ? 0 : 55 + v * 40).toString(16).padStart(2, '0');
    return '#' + lvl(r) + lvl(g) + lvl(b);
}

// 采样 xterm 当前渲染区，取众数 bg color → "#rrggbb" 或 null
// 密集采样：viewport 内每 step 个 cell 取一个，找占比最高的显式 bg；
// 还要求该 bg 占非默认 cell 的 ≥50%（避免被 statusline / 行号列等局部 hl 拐走）
function sampleDominantBg(term) {
    try {
        const buf = term.buffer.active;
        const rows = term.rows, cols = term.cols;
        if (!rows || !cols) return null;
        const counts = new Map();
        let total = 0, defaultCnt = 0;
        // 步长：保证总采样 ~200-400 个 cell，性能可控
        const stepY = Math.max(1, Math.floor(rows / 16));
        const stepX = Math.max(1, Math.floor(cols / 24));
        for (let y = 0; y < rows; y += stepY) {
            const line = buf.getLine(buf.viewportY + y);
            if (!line) continue;
            for (let x = 0; x < cols; x += stepX) {
                const cell = line.getCell(x);
                if (!cell) continue;
                total++;
                const mode = cell.getBgColorMode();
                const c = cell.getBgColor();
                if (mode === 0) { defaultCnt++; continue; }
                let hex;
                // xterm 颜色模式常量：CM_P16=1<<24, CM_P256=2<<24, CM_RGB=3<<24
                if (mode === (1 << 24) || mode === (2 << 24)) {
                    hex = paletteToHex(c);
                } else if (mode === (3 << 24)) {
                    // RGB: 颜色编码 = (R<<16)|(G<<8)|B
                    hex = '#' + ((c >>> 16) & 0xff).toString(16).padStart(2, '0')
                              + ((c >>> 8) & 0xff).toString(16).padStart(2, '0')
                              + (c & 0xff).toString(16).padStart(2, '0');
                } else continue;
                counts.set(hex, (counts.get(hex) || 0) + 1);
            }
        }
        if (!total) return null;
        // 如果 ≥60% cell 是 default mode，说明当前程序（如 shell / less 等）依赖终端默认色 → 返回 null
        // 让 xterm theme.background 决定，调用方负责保证 padding/host bg 与 theme.background 一致
        if (defaultCnt / total >= 0.6) return null;
        if (counts.size === 0) return null;
        let best = null, max = 0;
        for (const [k, v] of counts) if (v > max) { best = k; max = v; }
        // 主色需占非默认 cell 的 ≥50%，否则不可靠（例如 split window 不同颜色各占一半）
        const nonDefault = total - defaultCnt;
        if (nonDefault === 0 || max / nonDefault < 0.5) return null;
        return best;
    } catch (_) {
        return null;
    }
}

// OSC 11 payload → "#rrggbb"
// 支持 "rgb:RRRR/GGGG/BBBB"、"rgb:RR/GG/BB"、"#rrggbb"、"#rgb"
function parseOscColor(data) {
    if (!data) return null;
    const s = String(data).trim();
    if (s.startsWith('#')) {
        if (/^#[0-9a-fA-F]{6}$/.test(s)) return s.toLowerCase();
        if (/^#[0-9a-fA-F]{3}$/.test(s)) {
            return '#' + [...s.slice(1)].map(c => c + c).join('').toLowerCase();
        }
        return null;
    }
    const m = s.match(/^rgb:([0-9a-fA-F]+)\/([0-9a-fA-F]+)\/([0-9a-fA-F]+)$/);
    if (!m) return null;
    const toHex2 = (h) => {
        if (h.length === 0) return '00';
        // X11 spec: 1/2/3/4 hex digits, scale to 8-bit
        const v = parseInt(h, 16);
        const max = (1 << (h.length * 4)) - 1;
        const b = Math.round((v / max) * 255);
        return b.toString(16).padStart(2, '0');
    };
    return ('#' + toHex2(m[1]) + toHex2(m[2]) + toHex2(m[3])).toLowerCase();
}

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
                            const userSettings = getTerminalSettings();
                            const term = new Terminal({
                                cursorBlink: true,
                                cursorStyle: 'bar',
                                fontSize: userSettings.fontSize,
                                fontFamily: userSettings.fontFamily,
                                theme: {
                                    background: '#1e1e2e',
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
                    const userSettings = getTerminalSettings();
                    const term = new Terminal({
                        cursorBlink: true,
                        cursorStyle: 'bar',
                        fontSize: userSettings.fontSize,
                        fontFamily: userSettings.fontFamily,
                        theme: {
                            background: '#1e1e2e',
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

                    // 添加窗口大小调整事件监听器（去抖 + 存 cleanup）
                    // 之前这里注册的是未去抖、且无法移除的匿名监听器，每开一个新 session
                    // 就永久叠加一个，窗口缩放时 N 个 fit() 同时触发且永不回收。
                    const resizeHandler = debounce(() => {
                        if (fitAddon && term) {
                            fitAddon.fit();
                        }
                    }, 50);
                    window.addEventListener('resize', resizeHandler);
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

        const placeholder = document.getElementById('terminal-placeholder');
        if (placeholder) placeholder.classList.add('hidden');

        // 切回该 session 时恢复其上次报告的背景色（Ghostty extend 风格）
        // 设在 documentElement 上，所有 DOM 层都能继承
        if (entry.bgColor) {
            document.documentElement.style.setProperty('--term-bg', entry.bgColor);
        }

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

            const userSettings = getTerminalSettings();
            const termOptions = {
                cursorBlink: true,
                cursorStyle: 'bar',
                fontSize: userSettings.fontSize,
                fontFamily: userSettings.fontFamily,
                theme: {
                    background: '#1e1e2e',
                    foreground: '#FBF74B',
                    cursor: '#FBF74B'
                },
                allowTransparency: true,
                rendererType: 'canvas',
                blinkInterval: 500
            };

            const { term, fitAddon } = await this.createXterm(host, termOptions);

            // 自适应 padding 颜色：每次渲染采样实际 cell bg → 同步到 host/container
            // 让 padding 区域永远跟当前程序的背景色一致（vim/nvim 切主题也无缝）
            const applyBg = (color) => {
                if (!color) return;
                // 1) CSS 变量 --term-bg → padding / 父层 / scrollbar gutter 全部同步
                if (this.activeSessionId === sessionId) {
                    document.documentElement.style.setProperty('--term-bg', color);
                }
                host.style.setProperty('--term-bg', color);
                host.style.backgroundColor = color;
                // 2) xterm theme.background → 默认模式 cell（mode=0）也渲染成这个色
                //    避免 nvim 切主题时 padding 已变但 xterm 默认 cell 还是老色
                try {
                    const cur = term.options.theme || {};
                    if (cur.background !== color) {
                        term.options.theme = { ...cur, background: color };
                    }
                } catch (_) {}
                const e = this.terminals.get(sessionId);
                if (e) e.bgColor = color;
            };
            const renderTick = debounce(() => {
                const c = sampleDominantBg(term);
                if (c) applyBg(c);
            }, 60);
            try {
                term.onRender(renderTick);
            } catch (err) {
                console.warn('[initTerminal] onRender 订阅失败:', err);
            }

            // OSC 11（程序显式设置背景色）→ 优先级最高
            // 让"padding"颜色永远跟随当前程序的 bg，消除 vim/nvim 全屏时的可见黑边
            try {
                term.parser.registerOscHandler(11, (data) => {
                    const color = parseOscColor(data);
                    if (!color) return false;
                    try {
                        term.options.theme = { ...(term.options.theme || {}), background: color };
                    } catch (_) {}
                    applyBg(color);
                    return true;
                });
            } catch (err) {
                console.warn('[initTerminal] OSC 11 handler 注册失败:', err);
            }

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
    
    /**
     * 断开指定会话；如果是当前活跃会话，会清理终端 UI。
     * 切换连接不会触发此方法（切换只换前台显示）。
     */
    async disconnectSession(sessionId) {
        if (!sessionId) return;
        const isActive = window.currentSessionId === sessionId;

        // 当前在文件管理标签或正在传输文件时，断开前需确认（Tauri 原生 confirm 不可靠，走 plugin dialog）
        const fm = window.fileManager;
        const transferring = fm && fm.activeTransfers > 0;
        const onFileTab = window.activeTabId === 'file-manager';
        if (transferring || onFileTab) {
            const msg = transferring
                ? '当前有正在进行的文件传输，断开连接会中断传输。\n确定要断开吗？'
                : '当前正在使用文件管理，断开连接会关闭文件传输。\n确定要断开吗？';
            try {
                const ok = await window.api.dialog.confirm(msg, '断开连接');
                if (!ok) return;
            } catch (err) {
                console.error('[disconnectSession] confirm failed:', err);
                // 兜底：dialog 不可用时退回原生 confirm
                if (!confirm(msg)) return;
            }
        }

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

            window.uiManager.updateConnectionStatus(false);
            window.uiManager.updateServerInfo(false);

            // 文件管理器已随会话失效，回到终端标签引导用户重新连接其他服务器。
            window.uiManager.switchToTerminalTab();
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
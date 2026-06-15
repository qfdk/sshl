// terminal-manager.js
// 专门处理终端相关功能

import { getTerminalSettings } from './settings.js';

// Palette index → "#rrggbb"（xterm 256 色标准调色板）
const ANSI_BASE_16 = [
    '#000000','#cd3131','#0dbc79','#e5e510','#2472c8','#bc3fbc','#11a8cd','#e5e5e5',
    '#666666','#f14c4c','#23d18b','#f5f543','#3b8eea','#d670d6','#29b8db','#e5e5e5'
];
// 读取 xterm 运行时真实使用的 0-15 调色板（CanvasAddon 实际渲染用的就是它）。
// 设 theme.ansi 在本 xterm+CanvasAddon 下不生效（仍用其自带默认，如 magenta=#75507b），
// 故采样必须按「xterm 真画出来的色」解码，否则 paletteToHex 用 ANSI_BASE_16(#bc3fbc) →
// padding 与正文色不符露缝。私有 API：5.x 在 _themeService，4.x 在 _colorManager。
function getAnsiPalette(term) {
    try {
        const core = term?._core;
        const colors = (core?._themeService || core?._colorManager)?.colors;
        const ansi = colors?.ansi;
        if (Array.isArray(ansi) && ansi.length >= 16) {
            return ansi.slice(0, 16).map(c => String(c?.css || c).toLowerCase());
        }
    } catch (_) {}
    return null;
}

// pal：0-15 的实时调色板（缺省回退 ANSI_BASE_16）。16-255 双方算法一致，无需 pal。
function paletteToHex(idx, pal) {
    if (idx < 16) return (pal || ANSI_BASE_16)[idx];
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

// 终端原始默认背景色（createXtermInstance / initTerminal 的 theme.background）。
// 作为采样里 mode=0（默认背景）cell 的「权威基准色」，绝不被自适应采样自身改写——
// 否则一旦把 theme.background 染成某 TUI 的色（如 whiptail 紫），下一帧默认 cell 又被
// 当成那个色统计，自我强化锁死、永不回退（dpkg-reconfigure 后整屏变紫即此 bug）。
const DEFAULT_TERM_BG = '#1e1e2e';

// 单个 cell 的背景 → "#rrggbb"。mode=0（默认背景）返回 base（权威基准色），
// 其余按 xterm 颜色模式常量解码：CM_P16=1<<24, CM_P256=2<<24, CM_RGB=3<<24。
function cellBgHex(cell, base, pal) {
    const mode = cell.getBgColorMode();
    if (mode === 0) return base;
    const c = cell.getBgColor();
    if (mode === (1 << 24) || mode === (2 << 24)) return paletteToHex(c, pal);
    if (mode === (3 << 24)) {
        // RGB: 颜色编码 = (R<<16)|(G<<8)|B
        return '#' + ((c >>> 16) & 0xff).toString(16).padStart(2, '0')
                   + ((c >>> 8) & 0xff).toString(16).padStart(2, '0')
                   + (c & 0xff).toString(16).padStart(2, '0');
    }
    return null;
}

// 采样 xterm 渲染区「四条边缘」cell 的众数 bg → "#rrggbb" 或 null。
// padding 紧贴屏幕边缘，取边缘 cell 的背景最能消除色缝：
//   - 全屏 TUI（vim/nvim/htop）边缘即其根背景 → padding 完美贴合，切主题也无缝；
//   - 居中对话框（whiptail）不触边，四边纯净 → 不被中间对话框/装饰色带偏。
//     旧「全屏众数法」会被「正文一种紫 + 少量品红装饰 + 灰对话框」这类画面拐到错的主色，
//     导致 padding（品红）与正文（淡紫）色缝 —— 边缘取色根治之。
// mode=0（默认背景）cell 用 defaultBg（权威基准）代表，使 TUI 退出后能回退、不锁死。
function sampleEdgeBg(term, defaultBg) {
    try {
        const buf = term.buffer.active;
        const rows = term.rows, cols = term.cols;
        if (!rows || !cols) return null;
        const base = (defaultBg || DEFAULT_TERM_BG).toLowerCase();
        const pal = getAnsiPalette(term);  // xterm 实时调色板，解码 0-15 与渲染一致
        const counts = new Map();
        let total = 0;
        const sample = (x, y) => {
            const line = buf.getLine(buf.viewportY + y);
            if (!line) return;
            const cell = line.getCell(x);
            if (!cell) return;
            total++;
            const hex = cellBgHex(cell, base, pal);
            if (hex) counts.set(hex, (counts.get(hex) || 0) + 1);
        };
        // 上/下整行 + 左/右整列（四角各被计两次，权重微偏，可忽略）
        for (let x = 0; x < cols; x++) { sample(x, 0); sample(x, rows - 1); }
        for (let y = 0; y < rows; y++) { sample(0, y); sample(cols - 1, y); }
        if (!total || counts.size === 0) return null;
        let best = null, max = 0;
        for (const [k, v] of counts) if (v > max) { best = k; max = v; }
        // 边缘主色需占边缘采样 ≥50%，否则边缘色杂（如分屏不同底色）→ 交回调用方用基准色兜底
        if (!best || max / total < 0.5) return null;
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

// 密码提示行匹配：行尾以冒号（半角/全角）结束，且包含 password 或其本地化等价词。
// 命中示例：`[sudo] password for hubspot:`、`Password:`、`Mot de passe de hubspot :`。
const PASSWORD_PROMPT_RE = /(password|mot de passe|passwort|contraseña|пароль)[^\n]*[:：]\s*$/i;

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

// 统一构造 xterm 实例。调用方的 options 在最后展开，可覆盖默认 theme/透明度等。
// 注意：xterm 5.x 已移除 `rendererType` 构造选项（静默忽略），默认是 DOM renderer，
// 要 canvas renderer 必须显式 loadAddon(CanvasAddon)，见 loadCanvasRenderer。
function createXtermInstance(options) {
    const userSettings = getTerminalSettings();
    return new Terminal({
        cursorBlink: true,
        cursorStyle: 'bar',
        // canvas/webgl renderer 失焦时用 cursorInactiveStyle，默认 'outline'（空心框，看着像块状）。
        // Tauri WebView 里 document.hasFocus() 常判为 false → 一直走 inactive 样式，
        // 故显式设 'bar'，保证激活/失焦都是竖线。
        cursorInactiveStyle: 'bar',
        fontSize: userSettings.fontSize,
        fontFamily: userSettings.fontFamily,
        theme: {
            background: '#1e1e2e',
            foreground: '#f0f0f0',
            cursor: '#ffffff'
        },
        allowTransparency: false,
        ...options
    });
}

// 切换到 canvas renderer：DOM renderer 在 vim/tmux 整屏翻页（Ctrl+B/F）等高频重绘下
// 会撕裂/残影/卡顿，canvas renderer 修复之。CanvasAddon 必须在 term.open() 之后加载
// （它需要已挂载的 DOM）。加载失败时静默回退 DOM renderer，不影响基本可用性。
function loadCanvasRenderer(term) {
    try {
        term.loadAddon(new CanvasAddon.CanvasAddon());
    } catch (err) {
        console.warn('[terminal] canvas renderer 加载失败，回退 DOM renderer:', err);
    }
}

// canvas renderer 把行高量化成整数 px → 渲染区(canvas)比可用高度矮几~十几 px，
// 默认贴顶，底部多出一道空隙（与终端同色但位置突兀，用户感知为"底部多一行"）。
// 这里把渲染区精确贴底：强制 .xterm-screen 高度 = canvas 高度（覆盖 CSS 的 height:100%，
// 否则 screen 会被撑满到可用高度），再用 relative 把整段余量推到顶部，底部只剩正常 padding。
// 左右 padding 不受影响；xterm 用 screen 的真实 getBoundingClientRect 算鼠标坐标，
// relative 偏移已包含在 rect 内，贴底不影响选择/点击。
function alignScreenToBottom(host) {
    if (!host) return;
    const xt = host.querySelector('.xterm');
    const sc = host.querySelector('.xterm-screen');
    const cv = host.querySelector('.xterm-screen canvas');
    if (!xt || !sc || !cv) return;
    const padTop = parseFloat(getComputedStyle(xt).paddingTop) || 0;
    const padBot = parseFloat(getComputedStyle(xt).paddingBottom) || 0;
    const avail = xt.clientHeight - padTop - padBot;
    const cvH = cv.getBoundingClientRect().height;
    if (!cvH) return;
    const leftover = Math.max(0, Math.round(avail - cvH));
    sc.style.setProperty('height', cvH + 'px', 'important');
    sc.style.setProperty('position', 'relative', 'important');
    sc.style.setProperty('top', leftover + 'px', 'important');
}

// 窗口缩放时去抖 fit；存 cleanup 以便 dispose 时移除监听器，避免多 session 叠加泄漏。
function attachResizeHandler(term, fitAddon, sessionId) {
    const resizeHandler = debounce(() => {
        if (!fitAddon || !term) return;
        fitAddon.fit();
        // 关键：fit 重排 xterm 网格后，必须把新列/行数同步给后端 PTY（window_change），
        // 否则 vim 等全屏程序仍按旧 TIOCGWINSZ 尺寸重绘，滚动时 erase-line 清除范围
        // 与实际列数错位 → 旧帧残留、内容堆左侧（截图里的 gutter 重影/截断）。
        // 隐藏的后台终端容器无尺寸，proposeDimensions 返回空 → 自动跳过，不误改其 PTY。
        try {
            const d = fitAddon.proposeDimensions();
            if (d && d.cols && d.rows && sessionId && window.api?.ssh) {
                window.api.ssh.resize(sessionId, d.cols, d.rows).catch(() => {});
            }
        } catch (_) {}
    }, 50);
    window.addEventListener('resize', resizeHandler);
    term._resizeHandler = resizeHandler;
    term._cleanup = () => window.removeEventListener('resize', resizeHandler);
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
    async createXterm(container, options = {}, sessionId = null) {
        if (typeof container === 'string') {
            container = document.getElementById(container);
        }

        // term.open() 之后挂 fit + canvas renderer，再 fit 一次并注册缩放监听。
        const buildTerminal = (resolve, reject) => {
            try {
                const term = createXtermInstance(options);
                const fitAddon = new FitAddon.FitAddon();
                term.loadAddon(fitAddon);
                term.open(container);
                loadCanvasRenderer(term);
                // 强制 scrollBarWidth=0：滚动条已用 CSS 隐藏（不占位），FitAddon 默认仍会
                // 减去滚动条宽度（macOS 回退 8~15px）算列数，导致 canvas 没用满、右侧留缝。
                // 置 0 后列数按容器全宽计算，canvas 铺满，消除右侧 8px 缝与 1px 亮线。
                try { if (term._core?.viewport) term._core.viewport.scrollBarWidth = 0; } catch (_) {}
                fitAddon.fit();
                attachResizeHandler(term, fitAddon, sessionId);
                // 强制延迟以确保适当的大小
                setTimeout(() => fitAddon.fit(), 100);
                resolve({ term, fitAddon });
            } catch (error) {
                console.error('创建终端错误:', error);
                reject(error);
            }
        };

        // canvas renderer 也要就绪，否则首次走动态加载、二次走 else 分支时 CanvasAddon 缺失。
        if (!window.Terminal || !window.FitAddon || !window.CanvasAddon) {
            // 动态加载脚本：xterm → fit → canvas，全部就绪后再创建终端
            return new Promise((resolve, reject) => {
                const loadScript = (src, label) => new Promise((res, rej) => {
                    const el = document.createElement('script');
                    el.src = src;
                    el.onload = res;
                    el.onerror = () => {
                        console.error(`加载 ${label} 失败`);
                        rej(new Error(`Failed to load ${label}`));
                    };
                    document.head.appendChild(el);
                });

                // 加载样式
                const xtermStylesheet = document.createElement('link');
                xtermStylesheet.rel = 'stylesheet';
                xtermStylesheet.href = 'app://node_modules/xterm/css/xterm.css';
                document.head.appendChild(xtermStylesheet);

                loadScript('app://node_modules/xterm/lib/xterm.js', 'xterm.js')
                    .then(() => loadScript('app://node_modules/xterm-addon-fit/lib/xterm-addon-fit.js', 'xterm-addon-fit.js'))
                    .then(() => loadScript('app://node_modules/xterm-addon-canvas/lib/xterm-addon-canvas.js', 'xterm-addon-canvas.js'))
                    .then(() => buildTerminal(resolve, reject))
                    .catch(reject);
            });
        } else {
            // 脚本已加载，直接创建终端
            return new Promise((resolve, reject) => buildTerminal(resolve, reject));
        }
    }
    
    // 判断终端光标所在行是否为密码提示（sudo / ssh 等）。供「填充密码」按钮按需显隐。
    isPasswordPromptVisible(term) {
        if (!term) return false;
        try {
            const buf = term.buffer.active;
            const line = buf.getLine(buf.baseY + buf.cursorY);
            if (!line) return false;
            return PASSWORD_PROMPT_RE.test(line.translateToString(true));
        } catch (_) {
            return false;
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
                this._showExistingTerminal(sessionId, existing);
                return { term: existing.term, fitAddon: existing.fitAddon, isNew: false };
            }

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
                blinkInterval: 500
            };

            const { term, fitAddon } = await this.createXterm(host, termOptions, sessionId);

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
                // 取屏幕边缘众数色作 padding；mode=0 cell 用权威基准色（默认 DEFAULT_TERM_BG，
                // 程序经 OSC 11 显式改过则用其值）统计，使染色可回退、不锁死。
                // 边缘色杂（无 ≥50% 主色）时回退到基准色 —— 永不残留上一个程序的旧值。
                const baseBg = this.terminals.get(sessionId)?.defaultBg || DEFAULT_TERM_BG;
                applyBg(sampleEdgeBg(term, baseBg) || baseBg);
                alignScreenToBottom(host);
                // 仅活跃会话：根据光标行是否为密码提示，驱动「填充密码」按钮显隐
                if (this.activeSessionId === sessionId) {
                    try {
                        window.uiManager?.setFillPasswordPromptVisible(this.isPasswordPromptVisible(term));
                    } catch (_) {}
                }
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
                    // 程序显式声明的默认背景是权威值 → 同步基准色，使其作为 mode=0 代表，
                    // 不会被自适应采样回退（vim/nvim 靠 OSC 11 设主题时 padding 不被抢回默认色）。
                    const e = this.terminals.get(sessionId);
                    if (e) e.defaultBg = color;
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
                // 自适应背景采样的权威基准色：mode=0 cell 用它代表，仅 OSC 11 可改写
                defaultBg: DEFAULT_TERM_BG,
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
        // 断开前记录所属连接，供远程面板"重新连接"按钮使用（removeSession 后就取不到了）。
        const connectionId = window.sessionManager.getSession(sessionId)?.connectionId || null;

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

            // 不跳回终端：停留在当前标签，远程面板提示重新连接，本地面板仍可浏览。
            window.fileManager.clearFileManagerCache();
            window.fileManager.fileManagerInitialized = false;
            window.fileManager.renderRemoteEmptyState('连接已断开，请重新连接到服务器', connectionId);
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
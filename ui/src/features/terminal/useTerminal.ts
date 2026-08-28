import { useEffect, useRef } from 'react';
import { Terminal } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import { CanvasAddon } from 'xterm-addon-canvas';
import { Unicode11Addon } from 'xterm-addon-unicode11';
import { getActiveTabId, getCurrentSessionId, setCurrentSessionId, subscribe } from '../../../../assets/js/app-state.mjs';
import { getTerminalSettings } from '../../lib/terminal-settings';

type AppWindow = Window & {
  api?: any;
  fileManager?: any;
  connectionManager?: any;
  sessionManager?: any;
  uiManager?: any;
  terminalManager?: TerminalManager;
};

type TerminalEntry = {
  term: Terminal;
  fitAddon: FitAddon;
  host: HTMLDivElement;
  dataDisposer: (() => void) | null;
  cleanup: (() => void) | null;
  defaultBg: string;
  bgColor?: string;
};

export type TerminalManager = {
  readonly terminals: Map<string, TerminalEntry>;
  activeTerminal: Terminal | null;
  terminalFitAddon: FitAddon | null;
  activeSessionId: string | null;
  isTabSwitching: boolean;
  createXterm: (container: HTMLElement | string, options?: Record<string, unknown>, sessionId?: string | null) => Promise<{ term: Terminal; fitAddon: FitAddon }>;
  initTerminal: (sessionId: string, existingSession?: unknown, showBuffer?: boolean) => Promise<{ term: Terminal; fitAddon: FitAddon; isNew: boolean } | null>;
  getTerminalForSession: (sessionId: string) => Terminal | null;
  isPasswordPromptVisible: (term: Terminal | null) => boolean;
  disposeTerminalInstance: (sessionId: string) => void;
  disconnectSession: (sessionId: string) => Promise<void>;
  ensureTerminalVisible: () => void;
  resizeTerminal: () => void;
};

// 终端主题：与用户 Ghostty 配置（Carbonfox 改）对齐 —— 背景/前景/光标/选区 + 16 色调色板。
// 两条终端创建路径（createXtermInstance 默认 + initTerminal 的 termOptions）共用，
// 保证 options 展开覆盖 theme 时也不会丢调色板。
const TERM_THEME = {
  background: '#1d1d1d', foreground: '#f6f825', cursor: '#f2f4f8', cursorAccent: '#161616',
  selectionBackground: '#2a2a2a', selectionForeground: '#f2f4f8',
  black: '#282828', red: '#ee5396', green: '#00ff74', yellow: '#08bdba',
  blue: '#78a9ff', magenta: '#be95ff', cyan: '#33b1ff', white: '#dfdfe0',
  brightBlack: '#484848', brightRed: '#f16da6', brightGreen: '#46c880', brightYellow: '#2dc7c4',
  brightBlue: '#8cb6ff', brightMagenta: '#c8a5ff', brightCyan: '#52bdff', brightWhite: '#e4e4e5',
};

const ANSI_BASE_16 = [
  TERM_THEME.black, TERM_THEME.red, TERM_THEME.green, TERM_THEME.yellow,
  TERM_THEME.blue, TERM_THEME.magenta, TERM_THEME.cyan, TERM_THEME.white,
  TERM_THEME.brightBlack, TERM_THEME.brightRed, TERM_THEME.brightGreen, TERM_THEME.brightYellow,
  TERM_THEME.brightBlue, TERM_THEME.brightMagenta, TERM_THEME.brightCyan, TERM_THEME.brightWhite,
];

// 读取 xterm 运行时真实使用的 0-15 调色板（CanvasAddon 实际渲染用的就是它）。
// 设 theme.ansi 在本 xterm+CanvasAddon 下不生效（仍用其自带默认，如 magenta=#75507b），
// 故采样必须按「xterm 真画出来的色」解码，否则 paletteToHex 用 ANSI_BASE_16(#bc3fbc) →
// padding 与正文色不符露缝。私有 API：5.x 在 _themeService，4.x 在 _colorManager。
function getAnsiPalette(term: Terminal) {
  try {
    const core = (term as any)?._core;
    const colors = (core?._themeService || core?._colorManager)?.colors;
    const ansi = colors?.ansi;
    if (Array.isArray(ansi) && ansi.length >= 16) return ansi.slice(0, 16).map((c: any) => String(c?.css || c).toLowerCase());
  } catch (_) {}
  return null;
}

function paletteToHex(idx: number, pal: string[] | null) {
  if (idx < 16) return (pal || ANSI_BASE_16)[idx];
  if (idx >= 232) {
    const v = 8 + (idx - 232) * 10;
    const h = Math.min(255, v).toString(16).padStart(2, '0');
    return '#' + h + h + h;
  }
  const n = idx - 16;
  const r = Math.floor(n / 36), g = Math.floor((n % 36) / 6), b = n % 6;
  const lvl = (v: number) => (v === 0 ? 0 : 55 + v * 40).toString(16).padStart(2, '0');
  return '#' + lvl(r) + lvl(g) + lvl(b);
}

const DEFAULT_TERM_BG = TERM_THEME.background;

function cellBgHex(cell: any, base: string, pal: string[] | null) {
  const mode = cell.getBgColorMode();
  if (mode === 0) return base;
  const c = cell.getBgColor();
  if (mode === (1 << 24) || mode === (2 << 24)) return paletteToHex(c, pal);
  if (mode === (3 << 24)) {
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
function sampleEdgeBg(term: Terminal, defaultBg: string) {
  try {
    const buf = term.buffer.active;
    const rows = term.rows, cols = term.cols;
    if (!rows || !cols) return null;
    const base = (defaultBg || DEFAULT_TERM_BG).toLowerCase();
    const pal = getAnsiPalette(term);
    const counts = new Map<string, number>();
    let total = 0;
    const sample = (x: number, y: number) => {
      const line = buf.getLine(buf.viewportY + y);
      if (!line) return;
      const cell = line.getCell(x);
      if (!cell) return;
      total++;
      const hex = cellBgHex(cell, base, pal);
      if (hex) counts.set(hex, (counts.get(hex) || 0) + 1);
    };
    for (let x = 0; x < cols; x++) { sample(x, 0); sample(x, rows - 1); }
    for (let y = 0; y < rows; y++) { sample(0, y); sample(cols - 1, y); }
    if (!total || counts.size === 0) return null;
    let best: string | null = null, max = 0;
    for (const [key, value] of counts) if (value > max) { best = key; max = value; }
    if (!best || max / total < 0.5) return null;
    return best;
  } catch (_) {
    return null;
  }
}

// OSC 11 payload → "#rrggbb"
// 支持 "rgb:RRRR/GGGG/BBBB"、"rgb:RR/GG/BB"、"#rrggbb"、"#rgb"
function parseOscColor(data: unknown) {
  if (!data) return null;
  const s = String(data).trim();
  if (s.startsWith('#')) {
    if (/^#[0-9a-fA-F]{6}$/.test(s)) return s.toLowerCase();
    if (/^#[0-9a-fA-F]{3}$/.test(s)) return '#' + [...s.slice(1)].map(c => c + c).join('').toLowerCase();
    return null;
  }
  const m = s.match(/^rgb:([0-9a-fA-F]+)\/([0-9a-fA-F]+)\/([0-9a-fA-F]+)$/);
  if (!m) return null;
  const toHex2 = (h: string) => {
    if (h.length === 0) return '00';
    const v = parseInt(h, 16);
    const max = (1 << (h.length * 4)) - 1;
    return Math.round((v / max) * 255).toString(16).padStart(2, '0');
  };
  return ('#' + toHex2(m[1]) + toHex2(m[2]) + toHex2(m[3])).toLowerCase();
}

const PASSWORD_PROMPT_RE = /(password|mot de passe|passwort|contraseña|пароль)[^\n]*[:：]\s*$/i;

function debounce<T extends (...args: any[]) => void>(func: T, wait: number) {
  let timeout: ReturnType<typeof setTimeout>;
  return (...args: Parameters<T>) => {
    const later = () => { clearTimeout(timeout); func(...args); };
    clearTimeout(timeout);
    timeout = setTimeout(later, wait);
  };
}

// 统一构造 xterm 实例。调用方的 options 在最后展开，可覆盖默认 theme/透明度等。
// 注意：xterm 5.x 已移除 `rendererType` 构造选项（静默忽略），默认是 DOM renderer，
// 要 canvas renderer 必须显式 loadAddon(CanvasAddon)，见 loadCanvasRenderer。
function createXtermInstance(options: Record<string, unknown>) {
  const userSettings = getTerminalSettings();
  return new Terminal({
    cursorBlink: true,
    cursorStyle: 'bar',
    cursorInactiveStyle: 'bar',
    fontSize: userSettings.fontSize,
    fontFamily: userSettings.fontFamily,
    theme: { ...TERM_THEME },
    allowTransparency: false,
    // xterm 5.x 的 term.unicode（Unicode11Addon 注册入口）是 proposed API，
    // 不开这个开关 loadAddon 直接抛错。VS Code 同样常开。
    allowProposedApi: true,
    ...options,
  });
}

// CanvasAddon 必须在 term.open() 之后加载（它需要已挂载的 DOM）。加载失败时静默回退 DOM renderer。
function loadCanvasRenderer(term: Terminal) {
  try {
    term.loadAddon(new CanvasAddon());
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
function alignScreenToBottom(host: HTMLElement) {
  if (!host) return;
  const xt = host.querySelector('.xterm');
  const sc = host.querySelector('.xterm-screen');
  const cv = host.querySelector('.xterm-screen canvas');
  if (!xt || !sc || !cv) return;
  const padTop = parseFloat(getComputedStyle(xt).paddingTop) || 0;
  const padBot = parseFloat(getComputedStyle(xt).paddingBottom) || 0;
  const avail = (xt as HTMLElement).clientHeight - padTop - padBot;
  const cvH = cv.getBoundingClientRect().height;
  if (!cvH) return;
  const leftover = Math.max(0, Math.round(avail - cvH));
  (sc as HTMLElement).style.setProperty('height', cvH + 'px', 'important');
  (sc as HTMLElement).style.setProperty('position', 'relative', 'important');
  (sc as HTMLElement).style.setProperty('top', leftover + 'px', 'important');
}

// 窗口缩放时去抖 fit；存 cleanup 以便 dispose 时移除监听器，避免多 session 叠加泄漏。
function attachResizeHandler(term: Terminal, fitAddon: FitAddon, sessionId: string | null) {
  const resizeHandler = debounce(() => {
    if (!fitAddon || !term) return;
    fitAddon.fit();
    // 关键：fit 重排 xterm 网格后，必须把新列/行数同步给后端 PTY（window_change），
    // 否则 vim 等全屏程序仍按旧 TIOCGWINSZ 尺寸重绘，滚动时 erase-line 清除范围
    // 与实际列数错位 → 旧帧残留、内容堆左侧（截图里的 gutter 重影/截断）。
    // 隐藏的后台终端容器无尺寸，proposeDimensions 返回空 → 自动跳过，不误改其 PTY。
    try {
      const d = fitAddon.proposeDimensions();
      if (d && d.cols && d.rows && sessionId && (window as AppWindow).api?.ssh) {
        (window as AppWindow).api!.ssh.resize(sessionId, d.cols, d.rows).catch(() => {});
      }
    } catch (_) {}
  }, 50);
  window.addEventListener('resize', resizeHandler);
  (term as any)._resizeHandler = resizeHandler;
  (term as any)._cleanup = () => window.removeEventListener('resize', resizeHandler);
}

export function useTerminal(containerRef: React.RefObject<HTMLDivElement | null>) {
  const terminalsRef = useRef(new Map<string, TerminalEntry>());
  const activeTerminalRef = useRef<Terminal | null>(null);
  const terminalFitAddonRef = useRef<FitAddon | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);
  const isTabSwitchingRef = useRef(false);
  const resizeTerminalRef = useRef<() => void>(null);
  const managerRef = useRef<TerminalManager | null>(null);

  if (!resizeTerminalRef.current) {
    resizeTerminalRef.current = debounce(() => {
      if (terminalFitAddonRef.current && activeTerminalRef.current && getCurrentSessionId()) {
        try {
          terminalFitAddonRef.current.fit();
          const dimensions = terminalFitAddonRef.current.proposeDimensions();
          if (dimensions && dimensions.cols && dimensions.rows) {
            const api = (window as AppWindow).api;
            api?.ssh?.resize(getCurrentSessionId(), dimensions.cols, dimensions.rows).catch((err: any) => {
              if (!err.message || !err.message.includes('会话未找到')) console.error('调整终端大小失败:', err);
            });
          }
        } catch (err) {
          console.error('调整终端大小失败:', err);
        }
      }
    }, 100);
  }

  const createXterm = async (container: HTMLElement | string, options: Record<string, unknown> = {}, sessionId: string | null = null) => {
    if (typeof container === 'string') container = document.getElementById(container)!;
    // term.open() 之后挂 fit + canvas renderer，再 fit 一次并注册缩放监听。
    const buildTerminal = (resolve: (value: { term: Terminal; fitAddon: FitAddon }) => void, reject: (reason?: unknown) => void) => {
      try {
        const term = createXtermInstance(options);
        const fitAddon = new FitAddon();
        term.loadAddon(fitAddon);
        // Unicode 11 宽度表：默认 Unicode 6 把绝大多数 emoji 算成 1 格，Apple Color Emoji 字形实占 2 格。
        term.loadAddon(new Unicode11Addon());
        (term as any).unicode.activeVersion = '11';
        term.open(container as HTMLElement);
        loadCanvasRenderer(term);
        try { if ((term as any)._core?.viewport) (term as any)._core.viewport.scrollBarWidth = 0; } catch (_) {}
        fitAddon.fit();
        attachResizeHandler(term, fitAddon, sessionId);
        setTimeout(() => fitAddon.fit(), 100);
        resolve({ term, fitAddon });
      } catch (error) {
        console.error('创建终端错误:', error);
        reject(error);
      }
    };
    return new Promise<{ term: Terminal; fitAddon: FitAddon }>((resolve, reject) => buildTerminal(resolve, reject));
  };

  const isPasswordPromptVisible = (term: Terminal | null) => {
    if (!term) return false;
    try {
      const buf = term.buffer.active;
      const line = buf.getLine(buf.baseY + buf.cursorY);
      if (!line) return false;
      return PASSWORD_PROMPT_RE.test(line.translateToString(true));
    } catch (_) {
      return false;
    }
  };

  const getTerminalForSession = (sessionId: string) => terminalsRef.current.get(sessionId)?.term || null;
  const hideAllTerminalHosts = () => {
    for (const entry of terminalsRef.current.values()) if (entry.host) entry.host.style.display = 'none';
  };

  const showExistingTerminal = (sessionId: string, entry: TerminalEntry) => {
    hideAllTerminalHosts();
    entry.host.style.display = 'block';
    activeTerminalRef.current = entry.term;
    terminalFitAddonRef.current = entry.fitAddon;
    activeSessionIdRef.current = sessionId;
    if (entry.bgColor) document.documentElement.style.setProperty('--term-bg', entry.bgColor);
    requestAnimationFrame(() => {
      try { entry.term.focus(); } catch (err) { console.warn('[showTerminal] focus 失败:', err); }
      try {
        entry.fitAddon.fit();
        const d = entry.fitAddon.proposeDimensions();
        if (d && d.cols && d.rows && (window as AppWindow).api?.ssh) (window as AppWindow).api!.ssh.resize(sessionId, d.cols, d.rows).catch(() => {});
      } catch (err) { console.warn('[showTerminal] fit 失败:', err); }
    });
  };

  const initTerminal = async (sessionId: string, _existingSession: unknown = null, _showBuffer = false) => {
    try {
      const container = containerRef.current;
      if (!container) {
        console.error('找不到终端容器');
        return null;
      }
      if (!container.dataset.multiHostReady) {
        container.style.position = 'relative';
        container.dataset.multiHostReady = '1';
      }
      const existing = terminalsRef.current.get(sessionId);
      if (existing) {
        showExistingTerminal(sessionId, existing);
        return { term: existing.term, fitAddon: existing.fitAddon, isNew: false };
      }
      hideAllTerminalHosts();
      const host = document.createElement('div');
      host.className = 'term-host';
      host.dataset.sessionId = sessionId;
      host.style.position = 'absolute';
      host.style.inset = '0';
      container.appendChild(host);

      const userSettings = getTerminalSettings();
      const termOptions = {
        cursorBlink: true, cursorStyle: 'bar', fontSize: userSettings.fontSize, fontFamily: userSettings.fontFamily,
        theme: { ...TERM_THEME }, allowTransparency: true, blinkInterval: 500,
      };
      const { term, fitAddon } = await createXterm(host, termOptions, sessionId);

      const applyBg = (color: string | null) => {
        if (!color) return;
        if (activeSessionIdRef.current === sessionId) document.documentElement.style.setProperty('--term-bg', color);
        host.style.setProperty('--term-bg', color);
        host.style.backgroundColor = color;
        try {
          const cur = term.options.theme || {};
          if (cur.background !== color) term.options.theme = { ...cur, background: color };
        } catch (_) {}
        const entry = terminalsRef.current.get(sessionId);
        if (entry) entry.bgColor = color;
      };
      const renderTick = debounce(() => {
        const baseBg = terminalsRef.current.get(sessionId)?.defaultBg || DEFAULT_TERM_BG;
        applyBg(sampleEdgeBg(term, baseBg) || baseBg);
        alignScreenToBottom(host);
        if (activeSessionIdRef.current === sessionId) {
          try { (window as AppWindow).uiManager?.setFillPasswordPromptVisible(isPasswordPromptVisible(term)); } catch (_) {}
        }
      }, 60);
      try { term.onRender(renderTick); } catch (err) { console.warn('[initTerminal] onRender 订阅失败:', err); }

      try {
        term.parser.registerOscHandler(11, (data) => {
          const color = parseOscColor(data);
          if (!color) return false;
          try { term.options.theme = { ...(term.options.theme || {}), background: color }; } catch (_) {}
          const entry = terminalsRef.current.get(sessionId);
          if (entry) entry.defaultBg = color;
          applyBg(color);
          return true;
        });
      } catch (err) { console.warn('[initTerminal] OSC 11 handler 注册失败:', err); }

      let dataDisposer: (() => void) | null = null;
      try {
        const disposer = term.onData(data => {
          if ((window as AppWindow).api?.ssh) (window as AppWindow).api!.ssh.sendData(sessionId, data).catch((err: unknown) => console.error('发送数据失败:', err));
        });
        if (typeof disposer === 'function') dataDisposer = disposer;
        else if (disposer && typeof (disposer as any).dispose === 'function') dataDisposer = () => (disposer as any).dispose();
      } catch (err) { console.error('[initTerminal] 注册 onData 失败:', err); }

      const entry: TerminalEntry = { term, fitAddon, host, dataDisposer, defaultBg: DEFAULT_TERM_BG, cleanup: (term as any)._cleanup || null };
      terminalsRef.current.set(sessionId, entry);
      activeTerminalRef.current = term;
      terminalFitAddonRef.current = fitAddon;
      activeSessionIdRef.current = sessionId;

      requestAnimationFrame(() => {
        try { term.focus(); } catch (err) { console.warn('[initTerminal] focus 失败:', err); }
        try {
          fitAddon.fit();
          const d = fitAddon.proposeDimensions();
          if (d && d.cols && d.rows && (window as AppWindow).api?.ssh) (window as AppWindow).api!.ssh.resize(sessionId, d.cols, d.rows).catch((err: unknown) => console.error('初始化调整终端大小失败:', err));
        } catch (err) { console.warn('[initTerminal] fit 失败:', err); }
      });
      return { term, fitAddon, isNew: true };
    } catch (error) {
      console.error('初始化终端失败:', error);
      throw error;
    }
  };

  const disposeTerminalInstance = (sessionId: string) => {
    const entry = terminalsRef.current.get(sessionId);
    if (!entry) return;
    try { if (typeof entry.dataDisposer === 'function') entry.dataDisposer(); } catch (err) { console.warn('[dispose] dataDisposer 失败:', err); }
    try { if (typeof entry.cleanup === 'function') entry.cleanup(); } catch (err) { console.warn('[dispose] cleanup 失败:', err); }
    try { entry.term.dispose(); } catch (err) { console.warn('[dispose] term.dispose 失败:', err); }
    if (entry.host.parentNode) entry.host.parentNode.removeChild(entry.host);
    terminalsRef.current.delete(sessionId);
    if (activeSessionIdRef.current === sessionId) {
      activeTerminalRef.current = null;
      terminalFitAddonRef.current = null;
      activeSessionIdRef.current = null;
    }
  };

  const disconnectSession = async (sessionId: string) => {
    if (!sessionId) return;
    const isActive = getCurrentSessionId() === sessionId;
    const appWindow = window as AppWindow;
    const connectionId = appWindow.sessionManager?.getSession(sessionId)?.connectionId || null;
    const fm = appWindow.fileManager;
    const transferring = fm && fm.activeTransfers > 0;
    const onFileTab = getActiveTabId() === 'file-manager';
    if (transferring || onFileTab) {
      const msg = transferring ? '当前有正在进行的文件传输，断开连接会中断传输。\n确定要断开吗？' : '当前正在使用文件管理，断开连接会关闭文件传输。\n确定要断开吗？';
      try {
        const ok = await appWindow.api.dialog.confirm(msg, '断开连接');
        if (!ok) return;
      } catch (err) {
        console.error('[disconnectSession] confirm failed:', err);
        if (!confirm(msg)) return;
      }
    }
    try { await appWindow.api.ssh.disconnect(sessionId); } catch (error) { console.error('断开连接失败:', error); }
    appWindow.sessionManager.removeSession(sessionId);
    disposeTerminalInstance(sessionId);
    if (isActive) {
      setCurrentSessionId(null);
      appWindow.uiManager.updateConnectionStatus(false);
      appWindow.uiManager.updateServerInfo(false);
      appWindow.fileManager.clearFileManagerCache();
      appWindow.fileManager.fileManagerInitialized = false;
      appWindow.fileManager.renderRemoteEmptyState('连接已断开，请重新连接到服务器', connectionId);
    }
    await appWindow.connectionManager.loadConnections();
  };

  const ensureTerminalVisible = () => {
    if (terminalFitAddonRef.current && !isTabSwitchingRef.current) resizeTerminalRef.current!();
    if (activeTerminalRef.current && !isTabSwitchingRef.current) setTimeout(() => {
      try { activeTerminalRef.current?.focus(); } catch (err) { console.warn('聚焦终端失败:', err); }
    }, 100);
  };
  const resizeTerminal = () => resizeTerminalRef.current!();

  if (!managerRef.current) {
    managerRef.current = {
      get terminals() { return terminalsRef.current; },
      get activeTerminal() { return activeTerminalRef.current; },
      set activeTerminal(value) { activeTerminalRef.current = value; },
      get terminalFitAddon() { return terminalFitAddonRef.current; },
      set terminalFitAddon(value) { terminalFitAddonRef.current = value; },
      get activeSessionId() { return activeSessionIdRef.current; },
      set activeSessionId(value) { activeSessionIdRef.current = value; },
      get isTabSwitching() { return isTabSwitchingRef.current; },
      set isTabSwitching(value) { isTabSwitchingRef.current = value; },
      createXterm, initTerminal, getTerminalForSession, isPasswordPromptVisible,
      disposeTerminalInstance, disconnectSession, ensureTerminalVisible, resizeTerminal,
    };
  }

  useEffect(() => {
    const unsubscribe = subscribe('activeTabId', () => {
      if (getActiveTabId() !== 'terminal' || !activeTerminalRef.current) return;
      requestAnimationFrame(() => {
        isTabSwitchingRef.current = true;
        ensureTerminalVisible();
        setTimeout(() => {
          resizeTerminal();
          isTabSwitchingRef.current = false;
        }, 50);
      });
    });
    const container = containerRef.current;
    const observer = typeof ResizeObserver === 'undefined' || !container
      ? null
      : new ResizeObserver(() => {
        if (getActiveTabId() === 'terminal' && activeTerminalRef.current) resizeTerminal();
      });
    if (observer && container) observer.observe(container);
    return () => {
      unsubscribe();
      observer?.disconnect();
      for (const sessionId of [...terminalsRef.current.keys()]) disposeTerminalInstance(sessionId);
    };
  }, []);

  return managerRef.current;
}

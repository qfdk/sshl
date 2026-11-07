import { useEffect, useRef, type RefObject } from 'react';
import { pickEdgeBg } from './edge-bg';
import { centerOffset } from './screen-fit';
import { overlayOpen, shouldRefocusTerminal } from './refocus';
import type { Terminal } from 'xterm';
import type { FitAddon } from 'xterm-addon-fit';
import type { WebglAddon } from 'xterm-addon-webgl';
import { getActiveTabId, getCurrentSessionId, subscribe } from '../../lib/app-state';
import { flowAck } from '../../lib/flow-ack';
import { getTerminalSettings } from '../../lib/terminal-settings';

type AppWindow = Window & {
  api?: any;
  fileManager?: any;
  connectionManager?: any;
  sessionManager?: any;
  uiManager?: any;
  terminalManager?: TerminalManager;
};

type XtermModules = {
  Terminal: typeof import('xterm').Terminal;
  FitAddon: typeof import('xterm-addon-fit').FitAddon;
  CanvasAddon: typeof import('xterm-addon-canvas').CanvasAddon;
  WebglAddon: typeof import('xterm-addon-webgl').WebglAddon;
  Unicode11Addon: typeof import('xterm-addon-unicode11').Unicode11Addon;
};

type BuiltTerminal = { term: Terminal; fitAddon: FitAddon; webgl: WebglAddon | null };

type TerminalEntry = BuiltTerminal & {
  host: HTMLDivElement;
  dataDisposer: (() => void) | null;
  defaultBg: string;
  bgColor?: string;
};

type SpareTerminal = BuiltTerminal & { host: HTMLDivElement; fontSize: number; fontFamily: string };

export type TerminalManager = {
  readonly terminals: Map<string, TerminalEntry>;
  activeTerminal: Terminal | null;
  terminalFitAddon: FitAddon | null;
  activeSessionId: string | null;
  isTabSwitching: boolean;
  createXterm: (container: HTMLElement | string, options?: Record<string, unknown>) => Promise<BuiltTerminal>;
  initTerminal: (sessionId: string, existingSession?: unknown, showBuffer?: boolean) => Promise<{ term: Terminal; fitAddon: FitAddon; isNew: boolean } | null>;
  getTerminalForSession: (sessionId: string) => Terminal | null;
  isPasswordPromptVisible: (term: Terminal | null) => boolean;
  disposeTerminalInstance: (sessionId: string) => void;
  ensureTerminalVisible: () => void;
  resizeTerminal: () => void;
  prewarm: () => Promise<void>;
  proposeSize: () => { cols: number; rows: number } | null;
  syncPtySize: (sessionId: string) => void;
};

// xterm 与插件单独成块：首屏不再解析这约 400KB，首屏画出后空闲时预取（prewarm）。
// 这些包是 UMD/CJS，命名导出可能挂在 default 上，两处都取。
let xtermModules: Promise<XtermModules> | null = null;
const pick = (mod: any, name: string) => mod?.[name] ?? mod?.default?.[name];
export function loadXterm() {
  xtermModules ??= Promise.all([
    import('xterm'),
    import('xterm-addon-fit'),
    import('xterm-addon-canvas'),
    import('xterm-addon-webgl'),
    import('xterm-addon-unicode11'),
  ]).then(([xterm, fit, canvas, webgl, unicode11]) => ({
    Terminal: pick(xterm, 'Terminal'),
    FitAddon: pick(fit, 'FitAddon'),
    CanvasAddon: pick(canvas, 'CanvasAddon'),
    WebglAddon: pick(webgl, 'WebglAddon'),
    Unicode11Addon: pick(unicode11, 'Unicode11Addon'),
  }));
  return xtermModules;
}

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

// 读取 xterm 运行时真实使用的 0-15 调色板（渲染器实际用的就是它）。
// 设 theme.ansi 在本 xterm 下不生效（仍用其自带默认，如 magenta=#75507b），
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

// 采样 xterm 渲染区边缘 cell 的 bg → "#rrggbb" 或 null，取色规则见 pickEdgeBg。
// padding 紧贴屏幕边缘，取边缘 cell 的背景最能消除色缝：
//   - 全屏 TUI（vim/nvim/htop）边缘即其根背景 → padding 完美贴合，切主题也无缝；
//   - 居中对话框（whiptail）不触边，四边纯净 → 不被中间对话框/装饰色带偏。
//     旧「全屏众数法」会被「正文一种紫 + 少量品红装饰 + 灰对话框」这类画面拐到错的主色，
//     导致 padding（品红）与正文（淡紫）色缝 —— 边缘取色根治之。
// mode=0（默认背景）cell 用 defaultBg（权威基准）代表，使 TUI 退出后能回退、不锁死。
function sampleEdgeBg(term: Terminal, defaultBg: string) {
  try {
    const buf = term.buffer.active;
    const base = (defaultBg || DEFAULT_TERM_BG).toLowerCase();
    const pal = getAnsiPalette(term);
    return pickEdgeBg(term.cols, term.rows, (x, y) => {
      const cell = buf.getLine(buf.viewportY + y)?.getCell(x);
      return cell ? cellBgHex(cell, base, pal) : null;
    });
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
// GPU 渲染必须显式 loadAddon，见 loadGpuRenderer。
function createXtermInstance(mods: XtermModules, options: Record<string, unknown>) {
  const userSettings = getTerminalSettings();
  return new mods.Terminal({
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

// 渲染器必须在 term.open() 之后加载（需要已挂载的 DOM）。
// 首选 WebGL（GPU 绘制字形与背景）；不可用或 GPU 上下文丢失时回退 Canvas，再不行留 DOM renderer。
function loadCanvasRenderer(term: Terminal, mods: XtermModules) {
  try {
    term.loadAddon(new mods.CanvasAddon());
  } catch (err) {
    console.warn('[terminal] canvas renderer 加载失败，回退 DOM renderer:', err);
  }
}

function loadGpuRenderer(term: Terminal, mods: XtermModules): WebglAddon | null {
  try {
    const webgl = new mods.WebglAddon();
    webgl.onContextLoss(() => {
      releaseWebgl(webgl);
      loadCanvasRenderer(term, mods);
    });
    term.loadAddon(webgl);
    return webgl;
  } catch (err) {
    console.warn('[terminal] WebGL renderer 不可用，回退 canvas:', err);
    loadCanvasRenderer(term, mods);
    return null;
  }
}

// xterm 5.3 配套的 WebglAddon.dispose() 不释放 WebGL 上下文（xterm.js #6068），
// 频繁开关会话会把 WebKit 的活跃上下文配额耗尽，最老的终端随之丢上下文。先主动 loseContext。
function releaseWebgl(webgl: WebglAddon | null) {
  if (!webgl) return;
  try { (webgl as any)._renderer?._gl?.getExtension('WEBGL_lose_context')?.loseContext(); } catch (_) {}
  try { webgl.dispose(); } catch (_) {}
}

// 渲染器按整 cell 排版 → 渲染区(canvas)比可用区域小一点（不足一行 / 一列）。
// 余量堆在一侧就是一道更宽的边：贴顶时底部像多一行，贴底时顶部宽出最多一整行（vim 底色和 padding
// 不同色时就是一圈粗黑框）。这里把余量平分到上下、左右（同 Ghostty 的 window-padding-balance）：
// 强制 .xterm-screen 尺寸 = canvas 尺寸（覆盖 CSS 的 100%），再用 relative 偏移居中。
// xterm 用 screen 的真实 getBoundingClientRect 算鼠标坐标，偏移已包含在 rect 内，不影响选择/点击。
function centerScreenInPadding(host: HTMLElement) {
  if (!host) return;
  const xt = host.querySelector<HTMLElement>('.xterm');
  const sc = host.querySelector<HTMLElement>('.xterm-screen');
  const cv = host.querySelector('.xterm-screen canvas');
  if (!xt || !sc || !cv) return;
  const style = getComputedStyle(xt);
  const pad = (side: string) => parseFloat(style.getPropertyValue(`padding-${side}`)) || 0;
  const { width, height } = cv.getBoundingClientRect();
  if (!width || !height) return;
  sc.style.setProperty('width', width + 'px', 'important');
  sc.style.setProperty('height', height + 'px', 'important');
  sc.style.setProperty('position', 'relative', 'important');
  sc.style.setProperty('left', centerOffset(xt.clientWidth - pad('left') - pad('right'), width) + 'px', 'important');
  sc.style.setProperty('top', centerOffset(xt.clientHeight - pad('top') - pad('bottom'), height) + 'px', 'important');
}

// 字形度量依赖字体已加载：内置 Nerd Font 是 2MB 级 TTF，没加载完就 open 会按回退字体量 cell。
// 已加载时立即 resolve；最多等 500ms，不让异常字体卡住终端。
function waitForTerminalFont() {
  const { fontSize, fontFamily } = getTerminalSettings();
  if (!document.fonts?.load) return Promise.resolve();
  const loads = Promise.all([
    document.fonts.load(`${fontSize}px ${fontFamily}`),
    document.fonts.load(`bold ${fontSize}px ${fontFamily}`),
  ]).catch(() => {});
  return Promise.race([loads, new Promise(resolve => setTimeout(resolve, 500))]);
}

function makeHost(container: HTMLElement) {
  const host = document.createElement('div');
  host.className = 'term-host';
  host.style.position = 'absolute';
  host.style.inset = '0';
  container.appendChild(host);
  return host;
}

// PTY 尺寸同步的节流间隔：拖动窗口时每帧都重排本地网格，但给远端的 window_change 最多约 30 次/秒。
const PTY_RESIZE_INTERVAL_MS = 33;

export function useTerminal(containerRef: RefObject<HTMLDivElement | null>) {
  const terminalsRef = useRef(new Map<string, TerminalEntry>());
  const activeTerminalRef = useRef<Terminal | null>(null);
  const terminalFitAddonRef = useRef<FitAddon | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);
  const isTabSwitchingRef = useRef(false);
  const managerRef = useRef<TerminalManager | null>(null);
  const spareRef = useRef<SpareTerminal | null>(null);
  const sparePendingRef = useRef<Promise<void> | null>(null);
  const resizeFrameRef = useRef<number | null>(null);
  const ptySizeRef = useRef(new Map<string, string>());
  const ptyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const ptyLastSentRef = useRef(0);

  // 把 fit 后的网格尺寸同步给后端 PTY（window_change）——否则 vim 等全屏程序仍按旧 TIOCGWINSZ 重绘，
  // 滚动时 erase-line 范围与实际列数错位 → 旧帧残留、内容堆左侧。尺寸没变不重复发。
  // 隐藏的后台终端容器无尺寸，proposeDimensions 返回空 → 自动跳过，不误改其 PTY。
  const sendPtySize = (sessionId: string, fitAddon: FitAddon) => {
    try {
      const d = fitAddon.proposeDimensions();
      if (!d || !d.cols || !d.rows) return;
      const key = `${d.cols}x${d.rows}`;
      if (ptySizeRef.current.get(sessionId) === key) return;
      ptySizeRef.current.set(sessionId, key);
      ptyLastSentRef.current = performance.now();
      (window as AppWindow).api?.ssh?.resize(sessionId, d.cols, d.rows).catch((err: any) => {
        ptySizeRef.current.delete(sessionId);
        if (!err?.message || !err.message.includes('会话未找到')) console.error('调整终端大小失败:', err);
      });
    } catch (err) {
      console.error('调整终端大小失败:', err);
    }
  };

  const schedulePtySize = (sessionId: string, fitAddon: FitAddon) => {
    const wait = PTY_RESIZE_INTERVAL_MS - (performance.now() - ptyLastSentRef.current);
    if (wait <= 0) { sendPtySize(sessionId, fitAddon); return; }
    if (ptyTimerRef.current) clearTimeout(ptyTimerRef.current);
    ptyTimerRef.current = setTimeout(() => {
      ptyTimerRef.current = null;
      const id = activeSessionIdRef.current;
      if (id && terminalFitAddonRef.current) sendPtySize(id, terminalFitAddonRef.current);
    }, wait);
  };

  // 窗口/容器尺寸变化：每个动画帧重排一次（像原生终端那样拖动中实时跟随），并立刻把渲染区在 padding 内居中。
  const resizeNow = () => {
    const sessionId = activeSessionIdRef.current;
    const fitAddon = terminalFitAddonRef.current;
    if (!sessionId || !fitAddon || !activeTerminalRef.current || !getCurrentSessionId()) return;
    try {
      fitAddon.fit();
      const host = terminalsRef.current.get(sessionId)?.host;
      if (host) centerScreenInPadding(host);
      schedulePtySize(sessionId, fitAddon);
    } catch (err) {
      console.error('调整终端大小失败:', err);
    }
  };
  const resizeTerminal = () => {
    if (resizeFrameRef.current !== null) return;
    resizeFrameRef.current = requestAnimationFrame(() => {
      resizeFrameRef.current = null;
      resizeNow();
    });
  };

  // term.open() 之后挂 fit + GPU 渲染器，再 fit 一次。
  const createXterm = async (container: HTMLElement | string, options: Record<string, unknown> = {}): Promise<BuiltTerminal> => {
    if (typeof container === 'string') container = document.getElementById(container)!;
    const [mods] = await Promise.all([loadXterm(), waitForTerminalFont()]);
    try {
      const term = createXtermInstance(mods, options);
      const fitAddon = new mods.FitAddon();
      term.loadAddon(fitAddon);
      // Unicode 11 宽度表：默认 Unicode 6 把绝大多数 emoji 算成 1 格，Apple Color Emoji 字形实占 2 格。
      term.loadAddon(new mods.Unicode11Addon());
      (term as any).unicode.activeVersion = '11';
      term.open(container as HTMLElement);
      const webgl = loadGpuRenderer(term, mods);
      try { if ((term as any)._core?.viewport) (term as any)._core.viewport.scrollBarWidth = 0; } catch (_) {}
      fitAddon.fit();
      setTimeout(() => { try { fitAddon.fit(); } catch (_) {} }, 100);
      return { term, fitAddon, webgl };
    } catch (error) {
      console.error('创建终端错误:', error);
      throw error;
    }
  };

  const sessionTermOptions = () => {
    const userSettings = getTerminalSettings();
    return {
      cursorBlink: true, cursorStyle: 'bar', fontSize: userSettings.fontSize, fontFamily: userSettings.fontFamily,
      theme: { ...TERM_THEME }, allowTransparency: true, blinkInterval: 500,
    };
  };

  const disposeBuilt = (built: BuiltTerminal & { host: HTMLDivElement }) => {
    releaseWebgl(built.webgl);
    try { built.term.dispose(); } catch (err) { console.warn('[dispose] term.dispose 失败:', err); }
    if (built.host.parentNode) built.host.parentNode.removeChild(built.host);
  };

  // 预建一个备用终端（xterm 实例 + GPU 上下文 + 字形度量），隐藏在终端区里。
  // 双击连接时直接认领它，不在连接路径上现建；同时它的 fit 结果就是新 PTY 的初始尺寸。
  const buildSpare = async () => {
    const container = containerRef.current;
    if (!container || spareRef.current || !container.clientWidth || !container.clientHeight) return;
    const host = makeHost(container);
    host.style.visibility = 'hidden';
    host.style.pointerEvents = 'none';
    try {
      const { fontSize, fontFamily } = getTerminalSettings();
      const built = await createXterm(host, sessionTermOptions());
      spareRef.current = { ...built, host, fontSize, fontFamily };
    } catch (err) {
      console.warn('[terminal] 预建终端失败:', err);
      if (host.parentNode) host.parentNode.removeChild(host);
    }
  };

  const prewarm = () => {
    if (spareRef.current) return Promise.resolve();
    sparePendingRef.current ??= buildSpare().finally(() => { sparePendingRef.current = null; });
    return sparePendingRef.current;
  };

  const takeSpare = () => {
    const spare = spareRef.current;
    spareRef.current = null;
    if (!spare) return null;
    const { fontSize, fontFamily } = getTerminalSettings();
    if (spare.fontSize !== fontSize || spare.fontFamily !== fontFamily) {
      disposeBuilt(spare);
      return null;
    }
    spare.host.style.visibility = '';
    spare.host.style.pointerEvents = '';
    return spare;
  };

  const proposeSize = () => {
    const fitAddon = spareRef.current?.fitAddon || terminalFitAddonRef.current;
    try {
      const d = fitAddon?.proposeDimensions();
      return d && d.cols && d.rows ? { cols: d.cols, rows: d.rows } : null;
    } catch (_) {
      return null;
    }
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
        sendPtySize(sessionId, entry.fitAddon);
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
      if (!spareRef.current && sparePendingRef.current) await sparePendingRef.current;
      let built: (BuiltTerminal & { host: HTMLDivElement }) | null = takeSpare();
      if (!built) {
        const host = makeHost(container);
        built = { ...(await createXterm(host, sessionTermOptions())), host };
      }
      const { term, fitAddon, webgl, host } = built;
      host.dataset.sessionId = sessionId;

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
        centerScreenInPadding(host);
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

      const entry: TerminalEntry = { term, fitAddon, webgl, host, dataDisposer, defaultBg: DEFAULT_TERM_BG };
      terminalsRef.current.set(sessionId, entry);
      activeTerminalRef.current = term;
      terminalFitAddonRef.current = fitAddon;
      activeSessionIdRef.current = sessionId;

      try { term.focus(); } catch (err) { console.warn('[initTerminal] focus 失败:', err); }
      requestAnimationFrame(() => {
        try {
          fitAddon.fit();
          centerScreenInPadding(host);
          sendPtySize(sessionId, fitAddon);
        } catch (err) { console.warn('[initTerminal] fit 失败:', err); }
      });
      // 认领后立刻在空闲时补一个备用终端，给下一次连接用。
      setTimeout(() => { void prewarm(); }, 300);
      return { term, fitAddon, isNew: true };
    } catch (error) {
      console.error('初始化终端失败:', error);
      throw error;
    }
  };

  const disposeTerminalInstance = (sessionId: string) => {
    const entry = terminalsRef.current.get(sessionId);
    if (!entry) return;
    // 已写入、还没解析完的输出等不到 write 回调了：先替它们确认，否则终端通道的 SSH 窗口会少掉这一截
    flowAck.terminalGone(sessionId);
    try { if (typeof entry.dataDisposer === 'function') entry.dataDisposer(); } catch (err) { console.warn('[dispose] dataDisposer 失败:', err); }
    disposeBuilt(entry);
    terminalsRef.current.delete(sessionId);
    ptySizeRef.current.delete(sessionId);
    if (activeSessionIdRef.current === sessionId) {
      activeTerminalRef.current = null;
      terminalFitAddonRef.current = null;
      activeSessionIdRef.current = null;
    }
  };

  const focusTerminalFor = (reason: 'tab' | 'window') => {
    const term = activeTerminalRef.current;
    if (!term || !shouldRefocusTerminal({ reason, activeTab: getActiveTabId(), active: document.activeElement, dialogOpen: overlayOpen(document) })) return;
    try { term.focus(); } catch (err) { console.warn('聚焦终端失败:', err); }
  };

  const ensureTerminalVisible = () => {
    if (terminalFitAddonRef.current && !isTabSwitchingRef.current) resizeTerminal();
    if (activeTerminalRef.current && !isTabSwitchingRef.current) setTimeout(() => {
      try { activeTerminalRef.current?.focus(); } catch (err) { console.warn('聚焦终端失败:', err); }
    }, 100);
  };

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
      disposeTerminalInstance, ensureTerminalVisible, resizeTerminal, prewarm, proposeSize,
      syncPtySize: (sessionId: string) => {
        const entry = terminalsRef.current.get(sessionId);
        if (entry) sendPtySize(sessionId, entry.fitAddon);
      },
    };
  }

  useEffect(() => {
    const unsubscribe = subscribe('activeTabId', () => {
      if (getActiveTabId() !== 'terminal') return;
      if (!activeTerminalRef.current) { void prewarm(); return; }
      requestAnimationFrame(() => {
        isTabSwitchingRef.current = true;
        ensureTerminalVisible();
        // ensureTerminalVisible 在切标签期间不聚焦；切回终端标签时焦点要直接进终端，不然直接打字没反应
        focusTerminalFor('tab');
        setTimeout(() => {
          resizeTerminal();
          isTabSwitchingRef.current = false;
        }, 50);
      });
    });
    // 从别的应用切回窗口：焦点不在输入框里就交给终端
    const onWindowFocus = () => focusTerminalFor('window');
    window.addEventListener('focus', onWindowFocus);
    const container = containerRef.current;
    // 容器尺寸变化（含窗口缩放）统一走这里；备用终端也跟着重排，保证它给出的初始尺寸是准的。
    const observer = typeof ResizeObserver === 'undefined' || !container
      ? null
      : new ResizeObserver(() => {
        if (getActiveTabId() !== 'terminal') return;
        if (activeTerminalRef.current) resizeTerminal();
        const spare = spareRef.current;
        if (spare) requestAnimationFrame(() => { try { spare.fitAddon.fit(); } catch (_) {} });
      });
    if (observer && container) observer.observe(container);
    // 首屏画出后空闲时加载 xterm 并预建备用终端，不占启动关键路径。
    const idle = (window as any).requestIdleCallback
      ? (window as any).requestIdleCallback(() => { void prewarm(); }, { timeout: 1500 })
      : setTimeout(() => { void prewarm(); }, 300);
    return () => {
      unsubscribe();
      window.removeEventListener('focus', onWindowFocus);
      observer?.disconnect();
      if ((window as any).cancelIdleCallback) (window as any).cancelIdleCallback(idle); else clearTimeout(idle);
      if (resizeFrameRef.current !== null) cancelAnimationFrame(resizeFrameRef.current);
      if (ptyTimerRef.current) clearTimeout(ptyTimerRef.current);
      for (const sessionId of [...terminalsRef.current.keys()]) disposeTerminalInstance(sessionId);
      const spare = spareRef.current;
      spareRef.current = null;
      if (spare) disposeBuilt(spare);
    };
  }, []);

  return managerRef.current;
}

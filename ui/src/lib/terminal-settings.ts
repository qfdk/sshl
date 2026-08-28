const STORAGE_KEY = 'sshl.terminalSettings';

export const GROUP_NERD = '图标字体（Nerd Font）';
export const GROUP_MONO = '等宽字体';
export const DEFAULT_FONT_FAMILY = '"JetBrainsMono Nerd Font Mono", "Apple Color Emoji", Menlo, Monaco, monospace';

const BUNDLED_NERD = {
  label: 'JetBrainsMono Nerd Font (内置·含图标)',
  value: DEFAULT_FONT_FAMILY,
  group: GROUP_NERD,
};

const NERD_CANDIDATES = [
  'JetBrainsMono Nerd Font Mono', 'JetBrainsMono Nerd Font', 'FiraCode Nerd Font Mono',
  'FiraCode Nerd Font', 'Hack Nerd Font Mono', 'Hack Nerd Font', 'MesloLGM Nerd Font Mono',
  'MesloLGM Nerd Font', 'Iosevka Nerd Font Mono', 'Iosevka Nerd Font',
  'CaskaydiaCove Nerd Font Mono', 'CaskaydiaCove Nerd Font', 'Symbols Nerd Font Mono',
  'Symbols Nerd Font',
];

const MONO_CANDIDATES = [
  'Menlo', 'Monaco', 'SF Mono', 'JetBrains Mono', 'Fira Code', 'Cascadia Code',
  'Cascadia Mono', 'Hack', 'Iosevka', 'Source Code Pro', 'IBM Plex Mono', 'Roboto Mono',
  'Inconsolata', 'Ubuntu Mono', 'Anonymous Pro', 'PT Mono', 'Noto Sans Mono',
  'Courier New', 'Consolas',
];

export const DEFAULTS = { fontSize: 16, fontFamily: DEFAULT_FONT_FAMILY };
const LEGACY_FONT_MIGRATION: Record<string, string> = {
  '"JetBrainsMono Nerd Font Mono", Menlo, Monaco, monospace': DEFAULTS.fontFamily,
  '"JetBrainsMono Nerd Font Mono", monospace': DEFAULTS.fontFamily,
};
const CUSTOM_SENTINEL = '__custom__';
export const FONT_CHUNK_SIZE = 12;

export type TerminalSettings = typeof DEFAULTS;
export type FontPreset = { label: string; value: string; group: string };

type TerminalManagerLike = {
  terminals?: Map<string, { term: any; fitAddon: any }>;
};

type AppWindow = Window & { terminalManager?: TerminalManagerLike; api?: any };

function getAppWindow() {
  return window as AppWindow;
}

function yieldToBrowser() {
  return new Promise<void>(resolve => {
    if (typeof (window as any).requestIdleCallback === 'function') {
      (window as Window & { requestIdleCallback: (callback: () => void) => number }).requestIdleCallback(resolve);
    } else {
      requestAnimationFrame(() => resolve());
    }
  });
}

function detectInstalledFontsSync(candidates: string[]) {
  const baselines = ['sans-serif', 'serif', 'monospace'];
  const testText = 'mWiqxLZ_.,gXY0123 中文ABC';
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d');
  if (!ctx) return [];
  const baselineWidths = baselines.map(b => {
    ctx.font = `72px ${b}`;
    return ctx.measureText(testText).width;
  });
  return candidates.filter(name => baselines.some((b, i) => {
    ctx.font = `72px "${name}", ${b}`;
    return Math.abs(ctx.measureText(testText).width - baselineWidths[i]) > 0.5;
  }));
}

/** 字体检测按每片 12 个候选异步执行，避免首次打开设置时阻塞主线程。 */
export async function detectInstalledFonts(candidates: string[]) {
  const installed: string[] = [];
  for (let i = 0; i < candidates.length; i += FONT_CHUNK_SIZE) {
    installed.push(...detectInstalledFontsSync(candidates.slice(i, i + FONT_CHUNK_SIZE)));
    if (i + FONT_CHUNK_SIZE < candidates.length) await yieldToBrowser();
  }
  return installed;
}

function buildNerdStack(installedNerd: string[]) {
  const list = installedNerd.length ? installedNerd : NERD_CANDIDATES.slice(0, 4);
  return `${list.map(name => `"${name}"`).join(', ')}, "Noto Sans Mono CJK SC", monospace`;
}

export async function buildSystemFontPresets(): Promise<FontPreset[]> {
  const [installedMono, installedNerd] = await Promise.all([
    detectInstalledFonts(MONO_CANDIDATES),
    detectInstalledFonts(NERD_CANDIDATES),
  ]);
  const nerdFallback = buildNerdStack(installedNerd);
  const presets: FontPreset[] = [BUNDLED_NERD];
  for (const name of installedNerd) {
    presets.push({ label: `${name} (含图标)`, value: `"${name}", monospace`, group: GROUP_NERD });
  }
  for (const name of installedMono) {
    presets.push({ label: name, value: `"${name}", ${nerdFallback}`, group: GROUP_MONO });
  }
  presets.push({ label: 'monospace', value: 'monospace', group: GROUP_MONO });
  return presets;
}

let presetsPromise: Promise<FontPreset[]> | null = null;
export function loadSystemFontPresets() {
  if (!presetsPromise) presetsPromise = buildSystemFontPresets();
  return presetsPromise;
}

export function warmSystemFontPresets() {
  if (typeof window === 'undefined') return;
  if (typeof (window as any).requestIdleCallback === 'function') {
    (window as Window & { requestIdleCallback: (callback: () => void) => number }).requestIdleCallback(() => { void loadSystemFontPresets(); });
  } else {
    window.setTimeout(() => { void loadSystemFontPresets(); }, 0);
  }
}

function read(): TerminalSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw);
    const family = parsed.fontFamily || DEFAULTS.fontFamily;
    return {
      fontSize: Number(parsed.fontSize) || DEFAULTS.fontSize,
      fontFamily: LEGACY_FONT_MIGRATION[family] || family,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

let current = read();
export function getTerminalSettings() {
  return { ...current };
}

export function clampTerminalFontSize(value: unknown) {
  return Math.max(8, Math.min(40, Number(value) || DEFAULTS.fontSize));
}

function applyToAllTerminals(settings: TerminalSettings) {
  const tm = getAppWindow().terminalManager;
  if (!tm?.terminals) return;
  for (const [sessionId, entry] of tm.terminals.entries()) {
    try {
      entry.term.options.fontSize = settings.fontSize;
      entry.term.options.fontFamily = settings.fontFamily;
      const reflow = () => {
        if (!entry.fitAddon || !entry.term) return;
        try {
          if (entry.term._core?._renderService?.clear) {
            try { entry.term._core._renderService.clear(); } catch {}
          }
          entry.fitAddon.fit();
          entry.term.refresh(0, entry.term.rows - 1);
          const d = entry.fitAddon.proposeDimensions();
          if (d && d.cols && d.rows && getAppWindow().api?.ssh) {
            getAppWindow().api.ssh.resize(sessionId, d.cols, d.rows).catch(() => {});
          }
        } catch {}
      };
      const schedule = () => {
        requestAnimationFrame(reflow);
        setTimeout(reflow, 80);
      };
      if (document.fonts?.ready) {
        document.fonts.ready.then(schedule);
      } else {
        schedule();
      }
    } catch (err) {
      console.warn('[settings] 应用字体到 session 失败:', sessionId, err);
    }
  }
}

export function setTerminalSettings(next: Partial<TerminalSettings>, { persist = true } = {}) {
  current = {
    fontSize: clampTerminalFontSize(next.fontSize),
    fontFamily: next.fontFamily || DEFAULTS.fontFamily,
  };
  if (persist) localStorage.setItem(STORAGE_KEY, JSON.stringify(current));
  applyToAllTerminals(current);
  return current;
}

export { CUSTOM_SENTINEL };

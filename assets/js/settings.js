import { getSettingsDialogOpen, setSettingsDialogOpen } from './app-state.mjs';

// settings.js
// 终端字号 / 字体设置（localStorage 持久化 + 实时应用）

const STORAGE_KEY = 'sshl.terminalSettings';

// 分组标签 —— 下拉框 optgroup 用，顺序即展示顺序。
const GROUP_NERD = '图标字体（Nerd Font）';
const GROUP_MONO = '等宽字体';

const DEFAULT_FONT_FAMILY = '"JetBrainsMono Nerd Font Mono", "Apple Color Emoji", Menlo, Monaco, monospace';

// 内置（打包进 app）的 Nerd Font。@font-face 见 assets/css/fonts.css。
// 永远可用，不依赖系统安装，是「图标字体」分组的稳定锚点，也是 canvas 能可靠渲染图标的字体。
// "Apple Color Emoji" 必须排在任何系统等宽字体（含 monospace 泛型）之前：
// Menlo 等自带 ⚡ 之类的单色字形，排后面就永远轮不到彩色 emoji。
// 内置 NFM 已剔除 U+26A1 cmap 映射（见 fonts/ 构建说明），emoji 码点可正常落到彩色字体。
const BUNDLED_NERD = {
    label: 'JetBrainsMono Nerd Font (内置·含图标)',
    value: DEFAULT_FONT_FAMILY,
    group: GROUP_NERD
};

// Nerd Font 兜底栈：vim / starship / powerline 等需要的图标字形从这里补齐。
// 实际只追加已安装的 Nerd Font，由 initSettingsUI 在运行时探测后构建。
const NERD_CANDIDATES = [
    'JetBrainsMono Nerd Font Mono',
    'JetBrainsMono Nerd Font',
    'FiraCode Nerd Font Mono',
    'FiraCode Nerd Font',
    'Hack Nerd Font Mono',
    'Hack Nerd Font',
    'MesloLGM Nerd Font Mono',
    'MesloLGM Nerd Font',
    'Iosevka Nerd Font Mono',
    'Iosevka Nerd Font',
    'CaskaydiaCove Nerd Font Mono',
    'CaskaydiaCove Nerd Font',
    'Symbols Nerd Font Mono',
    'Symbols Nerd Font'
];

// 通用候选 monospace 字体 —— 仅展示实际装在系统里的那些。
const MONO_CANDIDATES = [
    'Menlo',
    'Monaco',
    'SF Mono',
    'JetBrains Mono',
    'Fira Code',
    'Cascadia Code',
    'Cascadia Mono',
    'Hack',
    'Iosevka',
    'Source Code Pro',
    'IBM Plex Mono',
    'Roboto Mono',
    'Inconsolata',
    'Ubuntu Mono',
    'Anonymous Pro',
    'PT Mono',
    'Noto Sans Mono',
    'Courier New',
    'Consolas'
];

/** Canvas 宽度差分法：用 sans-serif 作为基线，对比 "<font>, sans-serif" 的宽度是否变化。 */
function detectInstalledFonts(candidates) {
    const baselines = ['sans-serif', 'serif', 'monospace'];
    const testText = 'mWiqxLZ_.,gXY0123 中文ABC';
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d');
    const baselineWidths = baselines.map(b => {
        ctx.font = `72px ${b}`;
        return ctx.measureText(testText).width;
    });
    return candidates.filter(name => {
        return baselines.some((b, i) => {
            ctx.font = `72px "${name}", ${b}`;
            const w = ctx.measureText(testText).width;
            return Math.abs(w - baselineWidths[i]) > 0.5;
        });
    });
}

function buildNerdStack(installedNerd) {
    const list = installedNerd.length ? installedNerd : NERD_CANDIDATES.slice(0, 4);
    const quoted = list.map(n => `"${n}"`).join(', ');
    return `${quoted}, "Noto Sans Mono CJK SC", monospace`;
}

const DEFAULTS = {
    fontSize: 16,
    // 默认即内置 Nerd Font —— 新装用户开箱即有图标，无需手动选字体。
    fontFamily: DEFAULT_FONT_FAMILY
};

// 旧版默认栈（无 emoji 字体）→ 自动迁移到新栈，老用户无需重选字体。
const LEGACY_FONT_MIGRATION = {
    '"JetBrainsMono Nerd Font Mono", Menlo, Monaco, monospace': DEFAULTS.fontFamily,
    '"JetBrainsMono Nerd Font Mono", monospace': BUNDLED_NERD.value
};

/** 根据系统实际安装的字体动态生成 picker 选项。 */
export function buildSystemFontPresets() {
    const installedMono = detectInstalledFonts(MONO_CANDIDATES);
    const installedNerd = detectInstalledFonts(NERD_CANDIDATES);
    const nerdFallback = buildNerdStack(installedNerd);

    const presets = [BUNDLED_NERD];
    // 系统已安装的 Nerd Fonts（图标字体分组，内置项之后）
    for (const name of installedNerd) {
        presets.push({
            label: `${name} (含图标)`,
            value: `"${name}", monospace`,
            group: GROUP_NERD
        });
    }
    // 基础 monospace 字体（实际装的）
    for (const name of installedMono) {
        presets.push({
            label: name,
            value: `"${name}", ${nerdFallback}`,
            group: GROUP_MONO
        });
    }
    presets.push({ label: 'monospace', value: 'monospace', group: GROUP_MONO });
    return presets;
}

const CUSTOM_SENTINEL = '__custom__';

function read() {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return { ...DEFAULTS };
        const parsed = JSON.parse(raw);
        const family = parsed.fontFamily || DEFAULTS.fontFamily;
        return {
            fontSize: Number(parsed.fontSize) || DEFAULTS.fontSize,
            fontFamily: LEGACY_FONT_MIGRATION[family] || family
        };
    } catch {
        return { ...DEFAULTS };
    }
}

let current = read();

export function getTerminalSettings() {
    return { ...current };
}

function clampSize(n) {
    return Math.max(8, Math.min(40, Number(n) || DEFAULTS.fontSize));
}

// 应用到所有已创建的 xterm 实例
function applyToAllTerminals(settings) {
    const tm = window.terminalManager;
    if (!tm || !tm.terminals) return;
    for (const [sessionId, entry] of tm.terminals.entries()) {
        try {
            entry.term.options.fontSize = settings.fontSize;
            entry.term.options.fontFamily = settings.fontFamily;
            // 字体加载完毕后再 fit —— 否则 char 尺寸用旧值算 cols/rows
            const reflow = () => {
                if (!entry.fitAddon || !entry.term) return;
                try {
                    // xterm.js 缓存了 char 测量结果，强制清除以便重新测量新字体
                    if (entry.term._core?._renderService?.clear) {
                        try { entry.term._core._renderService.clear(); } catch {}
                    }
                    entry.fitAddon.fit();
                    entry.term.refresh(0, entry.term.rows - 1);
                    const d = entry.fitAddon.proposeDimensions();
                    if (d && d.cols && d.rows && window.api?.ssh) {
                        window.api.ssh.resize(sessionId, d.cols, d.rows).catch(() => {});
                    }
                } catch {}
            };
            // 两次 reflow：字体 ready 后立即一次，再延迟 80ms 兜底（覆盖 fonts.ready 提前解决的 case）
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

export function setTerminalSettings(next, { persist = true } = {}) {
    current = {
        fontSize: clampSize(next.fontSize),
        fontFamily: next.fontFamily || DEFAULTS.fontFamily
    };
    if (persist) {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(current));
    }
    applyToAllTerminals(current);
    return current;
}

export function initSettingsUI() {
    const openBtn = document.getElementById('open-settings-btn');
    const dialog = document.getElementById('settings-dialog');
    const closeBtn = document.getElementById('settings-close');
    const cancelBtn = document.getElementById('settings-cancel');
    const form = document.getElementById('settings-form');
    const fontSizeInput = document.getElementById('settings-font-size');
    const fontFamilySelect = document.getElementById('settings-font-family');
    const customWrap = document.getElementById('settings-custom-wrap');
    const customInput = document.getElementById('settings-font-family-custom');
    const preview = document.getElementById('settings-preview');
    const stepBtns = document.querySelectorAll('.font-size-step');

    if (!openBtn || !dialog || !form) return;

    // 占位填充 —— 真实列表在打开对话框时从系统读取
    let presets = buildSystemFontPresets();
    function renderOptions() {
        fontFamilySelect.innerHTML = '';
        // 按 group 字段聚成 optgroup，分组顺序按首次出现保留（图标字体在前，等宽在后）。
        const groups = new Map();
        for (const p of presets) {
            const key = p.group || GROUP_MONO;
            if (!groups.has(key)) groups.set(key, []);
            groups.get(key).push(p);
        }
        for (const [label, items] of groups) {
            const og = document.createElement('optgroup');
            og.label = label;
            for (const p of items) {
                const opt = document.createElement('option');
                opt.value = p.value;
                opt.textContent = p.label;
                og.appendChild(opt);
            }
            fontFamilySelect.appendChild(og);
        }
        const customGroup = document.createElement('optgroup');
        customGroup.label = '其他';
        const customOpt = document.createElement('option');
        customOpt.value = CUSTOM_SENTINEL;
        customOpt.textContent = '自定义…';
        customGroup.appendChild(customOpt);
        fontFamilySelect.appendChild(customGroup);
    }
    renderOptions();

    let snapshot = null;

    /** 把下拉框选中到给定 fontFamily；命中预设则选中，否则切到自定义。 */
    function selectFamily(family) {
        const matched = presets.find(p => p.value === family);
        if (matched) {
            fontFamilySelect.value = matched.value;
            customWrap.classList.add('hidden');
        } else {
            fontFamilySelect.value = CUSTOM_SENTINEL;
            customInput.value = family;
            customWrap.classList.remove('hidden');
        }
    }

    function readForm() {
        const size = clampSize(fontSizeInput.value);
        const sel = fontFamilySelect.value;
        const family = sel === CUSTOM_SENTINEL
            ? (customInput.value.trim() || DEFAULTS.fontFamily)
            : sel;
        return { fontSize: size, fontFamily: family };
    }

    function syncPreview(settings) {
        preview.style.setProperty('--preview-size', settings.fontSize + 'px');
        preview.style.setProperty('--preview-font', settings.fontFamily);
    }

    function applyLive() {
        const s = readForm();
        syncPreview(s);
        setTerminalSettings(s, { persist: false });
    }

    function openDialog() {
        // 立即弹窗，不等字体枚举（否则首次点击要卡几百 ms）。列表后台异步补全。
        snapshot = getTerminalSettings();
        fontSizeInput.value = snapshot.fontSize;
        selectFamily(snapshot.fontFamily);
        syncPreview(snapshot);
        setSettingsDialogOpen(true);
        document.dispatchEvent(new CustomEvent('settings:opened'));
        fontSizeInput.focus();
    }

    function closeDialog() {
        setSettingsDialogOpen(false);
        snapshot = null;
    }

    function revertAndClose() {
        // 没改过任何设置就直接关，跳过 revert 触发的全终端清缓存 + fit + refresh 重排
        if (snapshot) {
            const cur = getTerminalSettings();
            if (cur.fontSize !== snapshot.fontSize || cur.fontFamily !== snapshot.fontFamily) {
                setTerminalSettings(snapshot, { persist: false });
            }
        }
        closeDialog();
    }

    openBtn.addEventListener('click', openDialog);
    closeBtn.addEventListener('click', revertAndClose);
    cancelBtn.addEventListener('click', revertAndClose);
    document.addEventListener('settings:close-request', revertAndClose);
    document.addEventListener('keydown', (e) => {
        if (getSettingsDialogOpen() && e.key === 'Escape') {
            e.preventDefault();
            revertAndClose();
        }
    });

    // 实时联动
    fontSizeInput.addEventListener('input', applyLive);
    fontFamilySelect.addEventListener('change', () => {
        if (fontFamilySelect.value === CUSTOM_SENTINEL) {
            customWrap.classList.remove('hidden');
            customInput.focus();
        } else {
            customWrap.classList.add('hidden');
        }
        applyLive();
    });
    customInput.addEventListener('input', applyLive);

    stepBtns.forEach(btn => {
        btn.addEventListener('click', () => {
            const step = Number(btn.dataset.step) || 0;
            fontSizeInput.value = clampSize((Number(fontSizeInput.value) || DEFAULTS.fontSize) + step);
            applyLive();
        });
    });

    form.addEventListener('submit', (e) => {
        e.preventDefault();
        setTerminalSettings(readForm(), { persist: true });
        // 保存后清除 snapshot，不再 revert
        snapshot = null;
        setSettingsDialogOpen(false);
    });
}

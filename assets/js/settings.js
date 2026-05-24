// settings.js
// 终端字号 / 字体设置（localStorage 持久化 + 实时应用）

const STORAGE_KEY = 'sshl.terminalSettings';

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
    fontSize: 14,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace'
};

// 静态兜底列表 —— 当系统字体探测未启用时使用。
export const FONT_PRESETS = [
    { label: 'Menlo (默认)', value: 'Menlo, Monaco, "Courier New", monospace' },
    { label: 'Monaco', value: 'Monaco, Menlo, monospace' },
    { label: 'SF Mono', value: '"SF Mono", Menlo, monospace' },
    { label: 'monospace', value: 'monospace' }
];

/** 根据系统实际安装的字体动态生成 picker 选项。 */
export function buildSystemFontPresets() {
    const installedMono = detectInstalledFonts(MONO_CANDIDATES);
    const installedNerd = detectInstalledFonts(NERD_CANDIDATES);
    const nerdFallback = buildNerdStack(installedNerd);

    const presets = [];
    // 基础 monospace 字体（实际装的）
    for (const name of installedMono) {
        presets.push({
            label: name,
            value: `"${name}", ${nerdFallback}`
        });
    }
    // 已安装的 Nerd Fonts 单独列出（带图标）
    for (const name of installedNerd) {
        presets.push({
            label: `${name} (含图标)`,
            value: `"${name}", monospace`
        });
    }
    if (!presets.length) {
        // 系统什么都没探测到，回退静态列表
        return FONT_PRESETS.slice();
    }
    presets.push({ label: 'monospace', value: 'monospace' });
    return presets;
}

const CUSTOM_SENTINEL = '__custom__';

function read() {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return { ...DEFAULTS };
        const parsed = JSON.parse(raw);
        return {
            fontSize: Number(parsed.fontSize) || DEFAULTS.fontSize,
            fontFamily: parsed.fontFamily || DEFAULTS.fontFamily
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
        for (const p of presets) {
            const opt = document.createElement('option');
            opt.value = p.value;
            opt.textContent = p.label;
            fontFamilySelect.appendChild(opt);
        }
        const customOpt = document.createElement('option');
        customOpt.value = CUSTOM_SENTINEL;
        customOpt.textContent = '自定义…';
        fontFamilySelect.appendChild(customOpt);
    }
    renderOptions();

    /** 调用 Rust 端枚举系统字体目录。Core Text 返回的就是 CSS 可识别的 family name，直接信任。 */
    async function refreshFontListFromSystem() {
        try {
            const families = await window.api.file.listSystemFonts();
            console.log('[settings] system fonts:', families?.length, families);
            if (!Array.isArray(families) || !families.length) return;
            const nerd = families.filter(n => /nerd/i.test(n));
            const nerdFallback = nerd.length
                ? `${nerd.slice(0, 3).map(n => `"${n}"`).join(', ')}, monospace`
                : 'monospace';
            const fresh = [];
            // Nerd Font 置顶（含 VIM/Powerline 图标）
            for (const name of nerd) {
                fresh.push({ label: `${name} (含图标)`, value: `"${name}", monospace` });
            }
            // 其他字体（带 Nerd Font 兜底）
            for (const name of families) {
                if (nerd.includes(name)) continue;
                fresh.push({ label: name, value: `"${name}", ${nerdFallback}` });
            }
            fresh.push({ label: 'monospace', value: 'monospace' });
            presets = fresh;
            renderOptions();
        } catch (e) {
            console.warn('系统字体枚举失败，回退静态列表:', e);
        }
    }

    let snapshot = null;

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

    async function openDialog() {
        await refreshFontListFromSystem();
        snapshot = getTerminalSettings();
        fontSizeInput.value = snapshot.fontSize;
        const matched = presets.find(p => p.value === snapshot.fontFamily);
        if (matched) {
            fontFamilySelect.value = matched.value;
            customWrap.classList.add('hidden');
            customInput.value = '';
        } else {
            fontFamilySelect.value = CUSTOM_SENTINEL;
            customInput.value = snapshot.fontFamily;
            customWrap.classList.remove('hidden');
        }
        syncPreview(snapshot);
        dialog.classList.add('active');
        fontSizeInput.focus();
    }

    function closeDialog() {
        dialog.classList.remove('active');
        snapshot = null;
    }

    function revertAndClose() {
        if (snapshot) setTerminalSettings(snapshot, { persist: false });
        closeDialog();
    }

    openBtn.addEventListener('click', openDialog);
    closeBtn.addEventListener('click', revertAndClose);
    cancelBtn.addEventListener('click', revertAndClose);
    dialog.addEventListener('click', (e) => {
        if (e.target === dialog) revertAndClose();
    });
    document.addEventListener('keydown', (e) => {
        if (dialog.classList.contains('active') && e.key === 'Escape') {
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
        dialog.classList.remove('active');
    });
}

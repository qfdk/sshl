// settings.js
// 终端字号 / 字体设置（localStorage 持久化 + 实时应用）

const STORAGE_KEY = 'sshl.terminalSettings';

// 分组标签 —— 下拉框 optgroup 用，顺序即展示顺序。
const GROUP_NERD = '图标字体（Nerd Font）';
const GROUP_MONO = '等宽字体';

// 内置（打包进 app）的 Nerd Font。@font-face 见 assets/css/fonts.css。
// 永远可用，不依赖系统安装，是「图标字体」分组的稳定锚点，也是 canvas 能可靠渲染图标的字体。
const BUNDLED_NERD = {
    label: 'JetBrainsMono Nerd Font (内置·含图标)',
    value: '"JetBrainsMono Nerd Font Mono", monospace',
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
    fontSize: 14,
    // 默认即内置 Nerd Font —— 新装用户开箱即有图标，无需手动选字体。
    fontFamily: '"JetBrainsMono Nerd Font Mono", Menlo, Monaco, monospace'
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

    /** 分片跑 canvas 验证：系统字体常有几百个名字，一次性验证会阻塞主线程几百 ms，
     *  首次打开设置后点"取消"的卡顿就来自这里。每批之间让出事件循环，点击先响应。 */
    async function detectInstalledFontsChunked(candidates, chunkSize = 24) {
        const out = [];
        for (let i = 0; i < candidates.length; i += chunkSize) {
            out.push(...detectInstalledFonts(candidates.slice(i, i + chunkSize)));
            await new Promise(r => setTimeout(r, 0));
        }
        return out;
    }

    /** 调用 Rust 端枚举系统字体目录，再用 canvas 验证可渲染性后填充（Rust 文件名启发式可能含无效名）。 */
    async function refreshFontListFromSystem() {
        try {
            const raw = await window.api.file.listSystemFonts();
            if (!Array.isArray(raw) || !raw.length) return;
            // canvas 验证：只保留真正能被 canvas/浏览器渲染的 family 名。
            // Rust 文件名启发式会吐出系统不存在的拆分名（如 "Jet Brains Mono Nerd Font Mono"），
            // 直接信任会让用户选中后 fallback、图标显示为 □。canvas 渲染不出的名字一律剔除。
            const families = await detectInstalledFontsChunked(raw.filter(n => typeof n === 'string' && n.trim()));
            const nerd = families.filter(n => /nerd/i.test(n));
            // 内置字体永远在前；系统 Nerd Font 用内置 Mono 作兜底（系统名渲染不出时仍有图标）。
            const nerdFallback = '"JetBrainsMono Nerd Font Mono", monospace';
            const fresh = [BUNDLED_NERD];
            // 系统 Nerd Font（图标字体分组）
            for (const name of nerd) {
                if (name === 'JetBrainsMono Nerd Font Mono') continue; // 与内置同名，避免重复
                fresh.push({ label: `${name} (含图标)`, value: `"${name}", monospace`, group: GROUP_NERD });
            }
            // 其他等宽字体（带内置 Nerd Font 兜底，保证图标不丢）
            for (const name of families) {
                if (nerd.includes(name)) continue;
                fresh.push({ label: name, value: `"${name}", ${nerdFallback}`, group: GROUP_MONO });
            }
            fresh.push({ label: 'monospace', value: 'monospace', group: GROUP_MONO });
            // 重建下拉框后保留当前选中项（异步补全期间用户可能已在操作）
            const keep = dialog.classList.contains('active') ? readForm().fontFamily : null;
            presets = fresh;
            renderOptions();
            if (keep) selectFamily(keep);
        } catch (e) {
            console.warn('系统字体枚举失败，回退静态列表:', e);
        }
    }

    // 系统字体枚举较慢（Rust 枚举字体目录 + 逐个 canvas 验证），只跑一次并缓存。
    let fontListPromise = null;
    function ensureFontList() {
        if (!fontListPromise) fontListPromise = refreshFontListFromSystem();
        return fontListPromise;
    }

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
        dialog.classList.add('active');
        fontSizeInput.focus();
        ensureFontList();
    }

    function closeDialog() {
        dialog.classList.remove('active');
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

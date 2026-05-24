// settings.js
// 终端字号 / 字体设置（localStorage 持久化 + 实时应用）

const STORAGE_KEY = 'sshl.terminalSettings';

const DEFAULTS = {
    fontSize: 14,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace'
};

export const FONT_PRESETS = [
    { label: 'Menlo (默认)', value: 'Menlo, Monaco, "Courier New", monospace' },
    { label: 'Monaco', value: 'Monaco, Menlo, monospace' },
    { label: 'SF Mono', value: '"SF Mono", Menlo, monospace' },
    { label: 'Courier New', value: '"Courier New", Courier, monospace' },
    { label: 'Consolas', value: 'Consolas, "Courier New", monospace' },
    { label: 'JetBrains Mono', value: '"JetBrains Mono", Menlo, monospace' },
    { label: 'Fira Code', value: '"Fira Code", Menlo, monospace' },
    { label: 'monospace', value: 'monospace' }
];

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
            if (entry.fitAddon) {
                entry.fitAddon.fit();
                const d = entry.fitAddon.proposeDimensions();
                if (d && d.cols && d.rows && window.api?.ssh) {
                    window.api.ssh.resize(sessionId, d.cols, d.rows).catch(() => {});
                }
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

    // 填充下拉
    fontFamilySelect.innerHTML = '';
    for (const p of FONT_PRESETS) {
        const opt = document.createElement('option');
        opt.value = p.value;
        opt.textContent = p.label;
        fontFamilySelect.appendChild(opt);
    }
    const customOpt = document.createElement('option');
    customOpt.value = CUSTOM_SENTINEL;
    customOpt.textContent = '自定义…';
    fontFamilySelect.appendChild(customOpt);

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

    function openDialog() {
        snapshot = getTerminalSettings();
        fontSizeInput.value = snapshot.fontSize;
        const matched = FONT_PRESETS.find(p => p.value === snapshot.fontFamily);
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

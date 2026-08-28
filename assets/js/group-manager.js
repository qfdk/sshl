import { UNGROUPED_LABEL, normalizeGroupName, reorderGroups } from './connection-groups.mjs';

export function initGroupManager() {
    const dialog = document.getElementById('settings-dialog');
    const tab = document.querySelector('[data-settings-tab="groups"]');
    const panel = document.getElementById('settings-panel-groups');
    const form = document.getElementById('group-manager-form');
    const newGroupInput = document.getElementById('group-new-name');
    const addButton = document.getElementById('group-add-btn');
    const groupsContainer = document.getElementById('group-manager-groups');
    const connectionsContainer = document.getElementById('group-manager-connections');
    const tabs = document.querySelectorAll('[data-settings-tab]');
    const panels = document.querySelectorAll('[data-settings-panel]');

    if (!dialog || !tab || !panel || !form) return;

    let connections = [];
    let groups = [];
    let assignments = new Map();
    let loaded = false;
    let draggedGroup = null;

    function switchTab(name) {
        for (const button of tabs) {
            const active = button.dataset.settingsTab === name;
            button.classList.toggle('active', active);
            button.setAttribute('aria-selected', String(active));
        }
        for (const target of panels) target.hidden = target.dataset.settingsPanel !== name;
        if (name === 'groups') void load();
    }

    function render() {
        groupsContainer.innerHTML = '';
        if (!groups.length) {
            groupsContainer.innerHTML = '<div class="group-manager-empty">还没有分组，添加后可在下方快速分配机器。</div>';
        } else {
            for (const name of groups) {
                const row = document.createElement('div');
                row.className = 'group-manager-group-row';
                row.dataset.group = name;
                const handle = document.createElement('button');
                handle.type = 'button';
                handle.className = 'group-manager-drag-handle';
                handle.title = '拖拽调整分组顺序';
                handle.setAttribute('aria-label', `拖拽排序：${name}`);
                handle.innerHTML = window.Icons.svg('grip-vertical', 15, 2.25);
                const input = document.createElement('input');
                input.type = 'text';
                input.value = name;
                input.dataset.group = name;
                input.maxLength = 40;
                input.setAttribute('aria-label', `分组名称：${name}`);
                const count = document.createElement('span');
                count.className = 'group-manager-group-count';
                count.textContent = String(connections.filter(connection => normalizeGroupName(assignments.get(connection.id)) === name).length);
                const remove = document.createElement('button');
                remove.type = 'button';
                remove.className = 'icon-button group-manager-delete';
                remove.dataset.group = name;
                remove.title = '删除分组（机器移到默认列表）';
                remove.innerHTML = window.Icons.svg('trash-2', 14, 2.25);
                row.append(handle, input, count, remove);
                groupsContainer.appendChild(row);
            }
        }

        connectionsContainer.innerHTML = '';
        if (!connections.length) {
            connectionsContainer.innerHTML = '<div class="group-manager-empty">没有保存的机器。</div>';
            return;
        }
        for (const connection of connections) {
            const row = document.createElement('label');
            row.className = 'group-manager-connection-row';
            const name = document.createElement('span');
            name.className = 'group-manager-connection-name';
            name.textContent = connection.name || connection.host;
            const detail = document.createElement('span');
            detail.className = 'group-manager-connection-detail';
            detail.textContent = `${connection.username}@${connection.host}`;
            const select = document.createElement('select');
            select.dataset.id = connection.id;
            const ungrouped = document.createElement('option');
            ungrouped.value = '';
            ungrouped.textContent = UNGROUPED_LABEL;
            select.appendChild(ungrouped);
            for (const group of groups) {
                const option = document.createElement('option');
                option.value = group;
                option.textContent = group;
                select.appendChild(option);
            }
            select.value = assignments.get(connection.id) || '';
            select.addEventListener('change', () => assignments.set(connection.id, select.value));
            row.append(name, detail, select);
            connectionsContainer.appendChild(row);
        }
    }

    async function load() {
        if (loaded) return;
        try {
            const [connectionResult, groupResult] = await Promise.all([
                window.api.config.getConnections(),
                window.api.config.getConnectionGroups(),
            ]);
            connections = Array.isArray(connectionResult) ? connectionResult : [];
            groups = Array.isArray(groupResult) ? groupResult.map(normalizeGroupName).filter(Boolean) : [];
            assignments = new Map(connections.map(connection => [connection.id, normalizeGroupName(connection.group)]));
            loaded = true;
            render();
        } catch (error) {
            console.error('加载分组管理失败:', error);
            alert(`加载分组管理失败: ${error.message || error}`);
        }
    }

    for (const button of tabs) button.addEventListener('click', () => switchTab(button.dataset.settingsTab));
    document.addEventListener('settings:opened', () => switchTab('font'));

    addButton.addEventListener('click', () => {
        const name = newGroupInput.value.trim();
        if (!name) return newGroupInput.focus();
        if (groups.includes(name)) {
            alert('分组名称已存在');
            return;
        }
        groups.push(name);
        newGroupInput.value = '';
        render();
        newGroupInput.focus();
    });

    let groupDrag = null;

    const finishGroupDrag = event => {
        const drag = groupDrag;
        if (!drag || drag.pointerId !== event.pointerId) return;
        groupDrag = null;
        if (!drag.active) return;
        event.preventDefault();
        drag.row.classList.remove('dragging');
        groupsContainer.classList.remove('group-dragging');
        groupsContainer.querySelectorAll('.drag-over-before, .drag-over-after').forEach(element => {
            element.classList.remove('drag-over-before', 'drag-over-after');
        });
        drag.handle.releasePointerCapture?.(event.pointerId);
        if (event.type === 'pointercancel') {
            groups = drag.originalGroups;
        } else {
            groups = [...groupsContainer.querySelectorAll('.group-manager-group-row')].map(row => row.dataset.group);
        }
        render();
    };

    groupsContainer.addEventListener('pointerdown', event => {
        if (event.button !== 0) return;
        const handle = event.target.closest('.group-manager-drag-handle');
        const row = handle?.closest('.group-manager-group-row');
        if (!row) return;
        event.preventDefault();
        groupDrag = {
            handle,
            row,
            pointerId: event.pointerId,
            startY: event.clientY,
            active: false,
            originalGroups: [...groups],
        };
    });

    groupsContainer.addEventListener('pointermove', event => {
        const drag = groupDrag;
        if (!drag || drag.pointerId !== event.pointerId) return;
        if (!drag.active && Math.abs(event.clientY - drag.startY) < 6) return;
        if (!drag.active) {
            drag.active = true;
            drag.row.classList.add('dragging');
            groupsContainer.classList.add('group-dragging');
            drag.handle.setPointerCapture?.(event.pointerId);
        }
        event.preventDefault();
        const target = document.elementFromPoint(event.clientX, event.clientY)?.closest('.group-manager-group-row');
        if (!target || target === drag.row || !groupsContainer.contains(target)) return;
        const rect = target.getBoundingClientRect();
        const placeAfter = event.clientY >= rect.top + rect.height / 2;
        const reference = placeAfter ? target.nextElementSibling : target;
        if (reference !== drag.row) groupsContainer.insertBefore(drag.row, reference);
        groupsContainer.querySelectorAll('.drag-over-before, .drag-over-after').forEach(element => {
            element.classList.remove('drag-over-before', 'drag-over-after');
        });
        target.classList.add(placeAfter ? 'drag-over-after' : 'drag-over-before');
    });

    groupsContainer.addEventListener('pointerup', finishGroupDrag);
    groupsContainer.addEventListener('pointercancel', finishGroupDrag);
    window.addEventListener('pointerup', finishGroupDrag, true);
    window.addEventListener('pointercancel', finishGroupDrag, true);

    groupsContainer.addEventListener('change', event => {
        const input = event.target.closest('input[data-group]');
        if (!input) return;
        const oldName = input.dataset.group;
        const newName = input.value.trim();
        if (!newName || (newName !== oldName && groups.includes(newName))) {
            input.value = oldName;
            return;
        }
        groups[groups.indexOf(oldName)] = newName;
        for (const [id, group] of assignments) if (group === oldName) assignments.set(id, newName);
        render();
    });

    groupsContainer.addEventListener('click', event => {
        const button = event.target.closest('.group-manager-delete');
        if (!button) return;
        const name = button.dataset.group;
        groups = groups.filter(group => group !== name);
        for (const [id, group] of assignments) if (group === name) assignments.set(id, '');
        render();
    });

    form.addEventListener('submit', async event => {
        event.preventDefault();
        try {
            const layout = connections.map(connection => ({ id: connection.id, group: assignments.get(connection.id) || '' }));
            const result = await window.api.config.applyConnectionLayout(layout, groups);
            if (!result?.success && result?.success !== undefined) throw new Error(result.error || '保存失败');
            loaded = false;
            await window.connectionManager.loadConnections();
            await load();
        } catch (error) {
            console.error('保存分组失败:', error);
            alert(`保存分组失败: ${error.message || error}`);
        }
    });

    switchTab('font');
}

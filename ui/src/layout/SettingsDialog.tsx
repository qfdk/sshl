import { Check, Folder, GripVertical, Plus, Trash2, Type, X } from 'lucide-react';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { getSettingsDialogOpen, setSettingsDialogOpen, subscribe } from '../lib/app-state';
import { getTerminalSettings, clampTerminalFontSize, CUSTOM_SENTINEL, DEFAULT_FONT_FAMILY, loadSystemFontPresets, warmSystemFontPresets, setTerminalSettings, type FontPreset } from '../lib/terminal-settings';
import { normalizeGroupName, UNGROUPED_LABEL } from '../lib/connection-groups';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { Input } from '../components/ui/input';
import { Label } from '../components/ui/label';
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from '../components/ui/select';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs';
import { cn } from '../lib/utils';

const subscribeSettingsDialogOpen = (callback: () => void) => subscribe('settingsDialogOpen', callback);
const inputClassName = 'h-10 w-full';

type Connection = { id: string; name?: string; host?: string; username?: string; group?: string };
type AppWindow = Window & { api?: any; connectionManager?: any };

export function SettingsDialog() {
  const open = useSyncExternalStore(subscribeSettingsDialogOpen, getSettingsDialogOpen, () => false);
  const [fontSize, setFontSize] = useState(getTerminalSettings().fontSize);
  const [fontFamily, setFontFamily] = useState(getTerminalSettings().fontFamily);
  const [customFontFamily, setCustomFontFamily] = useState('');
  const [presets, setPresets] = useState<FontPreset[]>([]);
  const [groups, setGroups] = useState<string[]>([]);
  const [connections, setConnections] = useState<Connection[]>([]);
  const [assignments, setAssignments] = useState<Map<string, string>>(new Map());
  const [newGroupName, setNewGroupName] = useState('');
  // 拖动中只记录落点，松手才提交。原先在 pointermove 里直接改数据，列表会边拖边重排，
  // 鼠标下的行不断变化，很难放准。
  // 分组改动只存在本地，必须点保存才写入。没有脏标记时用户无从判断改动是否生效，
  // 关掉对话框还会静默丢失。
  const [savedSnapshot, setSavedSnapshot] = useState('');
  const [draggedGroup, setDraggedGroup] = useState<string | null>(null);
  const [groupDropBefore, setGroupDropBefore] = useState<string | null | undefined>(undefined);
  const groupDragRef = useRef<{ name: string; pointerId: number; startY: number; active: boolean } | null>(null);
  const groupDropRef = useRef<string | null | undefined>(undefined);
  const snapshotRef = useRef<ReturnType<typeof getTerminalSettings> | null>(null);

  useEffect(() => {
    warmSystemFontPresets();
  }, []);

  useEffect(() => {
    if (!open) return;
    const snapshot = getTerminalSettings();
    snapshotRef.current = snapshot;
    setFontSize(snapshot.fontSize);
    setFontFamily(snapshot.fontFamily);
    setCustomFontFamily(snapshot.fontFamily);
    void loadSystemFontPresets().then(setPresets);
  }, [open]);


  const snapshotOf = (map: Map<string, string>, list: string[]) =>
    JSON.stringify([[...map.entries()].sort(), list]);

  const loadGroups = async () => {
    try {
      const appWindow = window as AppWindow;
      const [connectionResult, groupResult] = await Promise.all([
        appWindow.api?.config.getConnections(),
        appWindow.api?.config.getConnectionGroups(),
      ]);
      const nextConnections = Array.isArray(connectionResult) ? connectionResult : [];
      const nextGroups = Array.isArray(groupResult) ? groupResult.map(normalizeGroupName).filter(Boolean) : [];
      const nextAssignments = new Map<string, string>(nextConnections.map((connection: Connection) => [connection.id, normalizeGroupName(connection.group)]));
      setConnections(nextConnections);
      setGroups(nextGroups);
      setAssignments(nextAssignments);
      setSavedSnapshot(snapshotOf(nextAssignments, nextGroups));
    } catch (error) {
      console.error('加载分组管理失败:', error);
      alert(`加载分组管理失败: ${(error as Error).message || error}`);
    }
  };

  const handleTabChange = (value: string) => {
    if (value === 'groups' && !connections.length && !groups.length) void loadGroups();
  };

  const closeWithoutSaving = () => {
    const snapshot = snapshotRef.current;
    if (snapshot) {
      const current = getTerminalSettings();
      if (current.fontSize !== snapshot.fontSize || current.fontFamily !== snapshot.fontFamily) {
        setTerminalSettings(snapshot, { persist: false });
      }
    }
    snapshotRef.current = null;
    setSettingsDialogOpen(false);
  };

  const selectedPreset = presets.some(preset => preset.value === fontFamily) ? fontFamily : CUSTOM_SENTINEL;
  const applyFontChange = (nextSize: number, nextFamily: string) => {
    const next = setTerminalSettings({ fontSize: nextSize, fontFamily: nextFamily }, { persist: false });
    setFontSize(next.fontSize);
    setFontFamily(next.fontFamily);
  };

  const handleFontSubmit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const family = selectedPreset === CUSTOM_SENTINEL ? (customFontFamily.trim() || DEFAULT_FONT_FAMILY) : selectedPreset;
    setTerminalSettings({ fontSize, fontFamily: family }, { persist: true });
    snapshotRef.current = null;
    setSettingsDialogOpen(false);
  };

  const addGroup = () => {
    const name = newGroupName.trim();
    if (!name) return;
    if (groups.includes(name)) {
      alert('分组名称已存在');
      return;
    }
    setGroups(previous => [...previous, name]);
    setNewGroupName('');
  };

  const renameGroup = (oldName: string, value: string) => {
    const newName = value.trim();
    if (!newName || (newName !== oldName && groups.includes(newName))) return;
    setGroups(previous => previous.map(group => group === oldName ? newName : group));
    setAssignments(previous => new Map([...previous].map(([id, group]) => [id, group === oldName ? newName : group])));
  };

  const removeGroup = (name: string) => {
    setGroups(previous => previous.filter(group => group !== name));
    setAssignments(previous => new Map([...previous].map(([id, group]) => [id, group === name ? '' : group])));
  };

  const setGroupDrop = (before: string | null | undefined) => {
    groupDropRef.current = before;
    setGroupDropBefore(before);
  };

  // 收尾挂 window：指针可能在容器外松开，否则拖拽状态会卡住。
  const finishGroupDrag = (event: PointerEvent) => {
    const drag = groupDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const before = groupDropRef.current;
    groupDragRef.current = null;
    setDraggedGroup(null);
    setGroupDrop(undefined);
    if (!drag.active || event.type === 'pointercancel' || before === undefined) return;
    moveGroup(drag.name, before);
  };

  useEffect(() => {
    window.addEventListener('pointerup', finishGroupDrag, true);
    window.addEventListener('pointercancel', finishGroupDrag, true);
    return () => {
      window.removeEventListener('pointerup', finishGroupDrag, true);
      window.removeEventListener('pointercancel', finishGroupDrag, true);
    };
  });

  const moveGroup = (name: string, targetName: string | null) => {
    if (name === targetName) return;
    setGroups(previous => {
      const next = previous.filter(group => group !== name);
      if (targetName === null) next.push(name);
      else next.splice(next.indexOf(targetName), 0, name);
      return next;
    });
  };

  const groupsDirty = savedSnapshot !== '' && savedSnapshot !== snapshotOf(assignments, groups);

  const saveGroups = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    try {
      const layout = connections.map(connection => ({ id: connection.id, group: assignments.get(connection.id) || '' }));
      const result = await (window as AppWindow).api.config.applyConnectionLayout(layout, groups);
      if (!result?.success && result?.success !== undefined) throw new Error(result.error || '保存失败');
      await (window as AppWindow).connectionManager.loadConnections();
      await loadGroups();
      setSettingsDialogOpen(false);
    } catch (error) {
      console.error('保存分组失败:', error);
      alert(`保存分组失败: ${(error as Error).message || error}`);
    }
  };

  return (
    <Dialog open={open} onOpenChange={nextOpen => nextOpen ? setSettingsDialogOpen(true) : closeWithoutSaving()}>
      <DialogContent id="settings-dialog" showCloseButton={false} className="max-w-3xl p-0">
        <DialogHeader className="flex-row items-center justify-between border-b border-border px-6 py-4">
          <DialogTitle className="text-xl">设置</DialogTitle>
          <DialogClose asChild>
            <Button type="button" id="settings-close" variant="ghost" size="icon" title="关闭"><X className="h-4 w-4" /><span className="sr-only">关闭</span></Button>
          </DialogClose>
        </DialogHeader>

        <Tabs defaultValue="font" onValueChange={handleTabChange} className="flex min-h-[26rem] flex-col sm:flex-row" orientation="vertical">
          <TabsList className="h-auto w-full shrink-0 justify-start gap-1 rounded-none border-b border-border bg-muted/40 p-3 sm:w-40 sm:flex-col sm:items-stretch sm:border-b-0 sm:border-r">
            <TabsTrigger value="font" data-settings-tab="font" className="justify-start gap-2"><Type className="h-4 w-4" />字体</TabsTrigger>
            <TabsTrigger value="groups" data-settings-tab="groups" className="justify-start gap-2"><Folder className="h-4 w-4" />分组管理</TabsTrigger>
          </TabsList>

          <div className="min-w-0 flex-1 p-6">
            <TabsContent value="font" id="settings-panel-font" data-settings-panel="font" className="mt-0 data-[state=inactive]:hidden">
              <Card className="border-border/80 shadow-sm">
                <CardHeader><CardTitle className="text-base">字体</CardTitle><CardDescription>调整终端字号和字体，预览会实时更新。</CardDescription></CardHeader>
                <form id="settings-form" onSubmit={handleFontSubmit}>
                  <CardContent className="grid gap-6">
                    <div className="grid gap-4 sm:grid-cols-[7rem_1fr]">
                      <div className="grid gap-2">
                        <Label htmlFor="settings-font-size">字号</Label>
                        <div className="flex h-10 items-center overflow-hidden rounded-md border border-input bg-background">
                          <Button type="button" variant="ghost" size="icon" className="h-full w-8 rounded-none" data-step="-1" aria-label="减小" onClick={() => applyFontChange(clampTerminalFontSize(fontSize - 1), fontFamily)}>−</Button>
                          <Input type="number" id="settings-font-size" className="h-full min-w-0 flex-1 border-0 px-1 text-center shadow-none focus-visible:ring-0" min={8} max={40} step={1} required value={fontSize} onChange={event => applyFontChange(Number(event.target.value), fontFamily)} />
                          <Button type="button" variant="ghost" size="icon" className="h-full w-8 rounded-none" data-step="1" aria-label="增大" onClick={() => applyFontChange(clampTerminalFontSize(fontSize + 1), fontFamily)}>+</Button>
                        </div>
                      </div>
                      <div className="grid gap-2">
                        <Label htmlFor="settings-font-family">字体</Label>
                        <Select value={selectedPreset} onValueChange={value => {
                          setFontFamily(value === CUSTOM_SENTINEL ? customFontFamily : value);
                          if (value === CUSTOM_SENTINEL) setCustomFontFamily(fontFamily);
                          else applyFontChange(fontSize, value);
                        }}>
                          <SelectTrigger id="settings-font-family"><SelectValue placeholder="选择字体" /></SelectTrigger>
                          <SelectContent>
                            {[...new Set(presets.map(preset => preset.group))].map(group => (
                              <SelectGroup key={group}><SelectLabel>{group}</SelectLabel>{presets.filter(preset => preset.group === group).map(preset => <SelectItem key={preset.value} value={preset.value}>{preset.label}</SelectItem>)}</SelectGroup>
                            ))}
                            <SelectGroup><SelectLabel>其他</SelectLabel><SelectItem value={CUSTOM_SENTINEL}>自定义…</SelectItem></SelectGroup>
                          </SelectContent>
                        </Select>
                      </div>
                    </div>
                    {selectedPreset === CUSTOM_SENTINEL && <div className="grid gap-2">
                      <Label htmlFor="settings-font-family-custom">自定义 fontFamily</Label>
                      <Input id="settings-font-family-custom" className={inputClassName} value={customFontFamily} onChange={event => { setCustomFontFamily(event.target.value); applyFontChange(fontSize, event.target.value || DEFAULT_FONT_FAMILY); }} placeholder='"Cascadia Code", monospace' autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                    </div>}
                    <div className="overflow-hidden rounded-lg bg-slate-950 p-4 font-mono text-sm leading-6 text-slate-100 shadow-inner" id="settings-preview" style={{ fontFamily: fontFamily, fontSize: `${fontSize}px` }}>
                      <div className="truncate">user@host:~$ ls -la /etc</div><div className="truncate text-slate-400">drwxr-xr-x  123 root  4096 May 24 10:42 .</div><div className="truncate">The quick brown fox 0123456789 → ✓</div><div className="truncate">Emoji: 😀 😃 🚀 🐱 ❤️ 🌈 🍎 ✅ ⚡ 中文</div><div className="truncate">Nerd Font 图标: &#xe0b0; &#xf07b; &#xf015; &#xf120; &#xe0a0; &#xf09b; &#xf135;</div>
                    </div>
                  </CardContent>
                  <DialogFooter className="border-t border-border px-6 py-4"><Button type="button" id="settings-cancel" variant="outline" onClick={closeWithoutSaving}>取消</Button><Button type="submit" id="settings-save"><Check className="h-4 w-4" />保存</Button></DialogFooter>
                </form>
              </Card>
            </TabsContent>

            <TabsContent value="groups" id="settings-panel-groups" data-settings-panel="groups" className="mt-0 data-[state=inactive]:hidden">
              <Card className="border-border/80 shadow-sm">
                <CardHeader><CardTitle className="text-base">分组管理</CardTitle><CardDescription>整理机器分组，拖拽连接列表即可调整顺序。</CardDescription></CardHeader>
                <form id="group-manager-form" onSubmit={saveGroups}>
                  <CardContent className="grid gap-6">
                    <div className="flex gap-2"><Input id="group-new-name" className="flex-1" placeholder="新分组名称" autoComplete="off" maxLength={40} value={newGroupName} onChange={event => setNewGroupName(event.target.value)} /><Button type="button" id="group-add-btn" onClick={addGroup} className="shrink-0"><Plus className="h-4 w-4" />添加分组</Button></div>
                    <div className="grid gap-2"><div className="text-sm font-medium">已有分组</div><div id="group-manager-groups" className="grid max-h-56 gap-2 overflow-y-auto pr-1">
                      {!groups.length && <div className="p-3 text-center text-xs text-muted-foreground">还没有分组，添加后可在下方快速分配机器。</div>}
                      {groups.map(name => {
                        const dropAbove = draggedGroup !== null && draggedGroup !== name && groupDropBefore === name;
                        const isLast = groups[groups.length - 1] === name;
                        const dropBelow = draggedGroup !== null && draggedGroup !== name && isLast && groupDropBefore === null;
                        return <div key={name} data-group-row={name} className={cn(
                          'relative flex items-center gap-2 rounded-md border border-border bg-card p-2 transition-opacity',
                          draggedGroup === name && 'opacity-45',
                          dropAbove && 'before:absolute before:inset-x-1 before:-top-1 before:h-0.5 before:rounded-full before:bg-primary',
                          dropBelow && 'after:absolute after:inset-x-1 after:-bottom-1 after:h-0.5 after:rounded-full after:bg-primary',
                        )}
                        onPointerMove={event => {
                          const drag = groupDragRef.current;
                          if (!drag || drag.pointerId !== event.pointerId) return;
                          // 6px 阈值：低于它算点击，不能一碰就开始排序
                          if (!drag.active && Math.abs(event.clientY - drag.startY) < 6) return;
                          if (!drag.active) { drag.active = true; setDraggedGroup(drag.name); }
                          if (drag.name === name) return;
                          const rect = event.currentTarget.getBoundingClientRect();
                          setGroupDrop(event.clientY >= rect.top + rect.height / 2 ? groups[groups.indexOf(name) + 1] ?? null : name);
                        }}>
                        <Button type="button" variant="ghost" size="icon" className="h-7 w-7 cursor-grab text-muted-foreground active:cursor-grabbing" aria-label={`拖拽排序：${name}`} onPointerDown={event => { event.preventDefault(); groupDragRef.current = { name, pointerId: event.pointerId, startY: event.clientY, active: false }; }}><GripVertical className="h-4 w-4" /></Button>
                        <Input defaultValue={name} maxLength={40} aria-label={`分组名称：${name}`} onBlur={event => renameGroup(name, event.target.value)} className="h-8 flex-1" />
                        <span className="min-w-6 text-right text-xs text-muted-foreground">{connections.filter(connection => normalizeGroupName(assignments.get(connection.id)) === name).length}</span>
                        <Button type="button" variant="ghost" size="icon" className="h-8 w-8 text-destructive" title="删除分组（机器移到默认列表）" onClick={() => removeGroup(name)}><Trash2 className="h-4 w-4" /></Button>
                      </div>;
                      })}
                    </div></div>
                    <div className="grid gap-2"><div className="text-sm font-medium">快速分配机器</div><div id="group-manager-connections" className="grid max-h-56 gap-2 overflow-y-auto pr-1">
                      {!connections.length && <div className="p-3 text-center text-xs text-muted-foreground">没有保存的机器。</div>}
                      {/* 这一行不能用 <label>：SelectTrigger 渲染成 <button>，属于可标记控件，
                          label 会把点击再转发给它——直接点下拉框会触发两次，弹开又立刻关闭，
                          点行内空白处也会莫名弹开。 */}
                      {connections.map(connection => <div key={connection.id} className="flex min-h-10 items-center gap-2 rounded-md border border-border bg-card p-2"><span className="min-w-24 truncate text-sm font-medium" title={connection.name || connection.host}>{connection.name || connection.host}</span><span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground" title={`${connection.username}@${connection.host}`}>{connection.username}@{connection.host}</span><Select value={assignments.get(connection.id) || '__ungrouped__'} onValueChange={value => setAssignments(previous => new Map(previous).set(connection.id, value === '__ungrouped__' ? '' : value))}><SelectTrigger className="w-32 shrink-0" aria-label={`分组：${connection.name || connection.host}`}><SelectValue /></SelectTrigger><SelectContent><SelectItem value="__ungrouped__">{UNGROUPED_LABEL}</SelectItem>{groups.map(group => <SelectItem key={group} value={group}>{group}</SelectItem>)}</SelectContent></Select></div>)}
                    </div></div>
                  </CardContent>
                  <DialogFooter className="items-center gap-3 border-t border-border px-6 py-4 sm:justify-between">
                    <span className="text-xs text-muted-foreground">{groupsDirty ? '有未保存的改动' : '改动已保存'}</span>
                    <Button type="submit" id="group-manager-save" disabled={!groupsDirty}><Check className="h-4 w-4" />保存</Button>
                  </DialogFooter>
                </form>
              </Card>
            </TabsContent>
          </div>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

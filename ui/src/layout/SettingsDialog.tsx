import { Check, Folder, GripVertical, Plus, RotateCcw, Trash2, Type, X } from 'lucide-react';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { getSettingsDialogOpen, setSettingsDialogOpen, subscribe } from '../lib/app-state';
import { getTerminalSettings, clampTerminalFontSize, CUSTOM_SENTINEL, DEFAULT_FONT_FAMILY, DEFAULTS, loadSystemFontPresets, warmSystemFontPresets, setTerminalSettings, type FontPreset } from '../lib/terminal-settings';
import { moveConnection, normalizeGroupName } from '../lib/connection-groups';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { Input } from '../components/ui/input';
import { Label } from '../components/ui/label';
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from '../components/ui/select';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs';
import { cn } from '../lib/utils';
import { GroupTree } from '../features/connections/GroupTree';

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
  const [newGroupName, setNewGroupName] = useState('');
  // 拖动中只记录落点，松手才提交。原先在 pointermove 里直接改数据，列表会边拖边重排，
  // 鼠标下的行不断变化，很难放准。
  // 分组改动只存在本地，必须点保存才写入。没有脏标记时用户无从判断改动是否生效，
  // 关掉对话框还会静默丢失。
  const [savedSnapshot, setSavedSnapshot] = useState('');
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


  const snapshotOf = (list: Connection[], groupList: string[]) =>
    JSON.stringify([list.map(connection => [connection.id, normalizeGroupName(connection.group)]), groupList]);

  const loadGroups = async () => {
    try {
      const appWindow = window as AppWindow;
      const [connectionResult, groupResult] = await Promise.all([
        appWindow.api?.config.getConnections(),
        appWindow.api?.config.getConnectionGroups(),
      ]);
      const nextConnections = Array.isArray(connectionResult) ? connectionResult : [];
      const nextGroups = Array.isArray(groupResult) ? groupResult.map(normalizeGroupName).filter(Boolean) : [];
      setConnections(nextConnections);
      setGroups(nextGroups);
      setSavedSnapshot(snapshotOf(nextConnections, nextGroups));
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

  // 只回到默认值并刷新预览，不落盘 —— 和面板里其他改动一样，要按保存才生效，
  // 这样误点还能直接关掉不管。
  const resetFontDefaults = () => {
    setCustomFontFamily('');
    applyFontChange(DEFAULTS.fontSize, DEFAULTS.fontFamily);
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
    // 名字没变就什么都别做。这是 onBlur 触发的：点下拉框会让输入框失焦，若照样
    // setState，整列表会重渲染并打断 Select 的展开。
    if (!newName || newName === oldName) return;
    if (groups.includes(newName)) return;
    setGroups(previous => previous.map(group => group === oldName ? newName : group));
    setConnections(previous => previous.map(connection => normalizeGroupName(connection.group) === oldName ? { ...connection, group: newName } : connection));
  };

  const removeGroup = (name: string) => {
    setGroups(previous => previous.filter(group => group !== name));
    setConnections(previous => previous.map(connection => normalizeGroupName(connection.group) === name ? { ...connection, group: '' } : connection));
  };

  const groupsDirty = savedSnapshot !== '' && savedSnapshot !== snapshotOf(connections, groups);

  const saveGroups = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    try {
      const layout = connections.map(connection => ({ id: connection.id, group: normalizeGroupName(connection.group) }));
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
                <CardHeader className="flex-row items-start justify-between gap-4 space-y-0">
                  <div className="grid gap-1.5">
                    <CardTitle className="text-base">字体</CardTitle>
                    <CardDescription>调整终端字号和字体，预览会实时更新。</CardDescription>
                  </div>
                  <Button type="button" id="settings-reset" variant="outline" size="sm" className="shrink-0" onClick={resetFontDefaults}>
                    <RotateCcw className="h-3.5 w-3.5" />还原默认
                  </Button>
                </CardHeader>
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
                  <DialogFooter className="border-t border-border px-6 py-4"><Button type="submit" id="settings-save"><Check className="h-4 w-4" />保存</Button></DialogFooter>
                </form>
              </Card>
            </TabsContent>

            <TabsContent value="groups" id="settings-panel-groups" data-settings-panel="groups" className="mt-0 data-[state=inactive]:hidden">
              <Card className="border-border/80 shadow-sm">
                <CardHeader><CardTitle className="text-base">分组管理</CardTitle><CardDescription>整理机器分组，拖拽连接列表即可调整顺序。</CardDescription></CardHeader>
                <form id="group-manager-form" onSubmit={saveGroups}>
                  <CardContent className="grid gap-6">
                    <div className="flex gap-2"><Input id="group-new-name" className="flex-1" placeholder="新分组名称" autoComplete="off" maxLength={40} value={newGroupName} onChange={event => setNewGroupName(event.target.value)} /><Button type="button" id="group-add-btn" onClick={addGroup} className="shrink-0"><Plus className="h-4 w-4" />添加分组</Button></div>
                    <div className="grid gap-2">
                      <div className="flex items-baseline justify-between">
                        <div className="text-sm font-medium">分组与机器</div>
                        <div className="text-xs text-muted-foreground">拖动机器归组，拖动把手排序分组</div>
                      </div>
                      <GroupTree
                        connections={connections}
                        groups={groups}
                        onMoveConnection={(id, group, beforeId) => setConnections(previous => moveConnection(previous, groups, id, group, beforeId) as Connection[])}
                        onReorderGroups={setGroups}
                        onRenameGroup={renameGroup}
                        onRemoveGroup={removeGroup}
                      />
                    </div>
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

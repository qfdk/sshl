// 访达工具栏上的自定义按钮：加、去掉、查有没有（launchers.rs 调：osascript -l JavaScript -e 本文件 动作 按钮路径 [偏好设置域]）。
// 访达把工具栏存在 com.apple.finder 的「NSToolbar Configuration Browser」里：TB Item Identifiers 是从左到右的一排，
// 自定义按钮的标识是 "com.apple.finder.loc "（带一个空格），它指向哪个文件记在 TB Item Plists 里，键是它在那一排里的位置。
// 只动我们那一项，别的按钮（包括它们的 alias 数据）原样留着。改完要重启访达才生效（launchers.rs 做）。
ObjC.import('Foundation');

const KEY = 'NSToolbar Configuration Browser';
const LOC = 'com.apple.finder.loc ';
// 用户从没改过工具栏时访达用的那一排（同访达自己记的 TB Default Item Identifiers）
const FALLBACK = ['com.apple.finder.BACK', 'com.apple.finder.SWCH', 'NSToolbarSpaceItem', 'com.apple.finder.ARNG', 'NSToolbarSpaceItem',
  'com.apple.finder.SHAR', 'com.apple.finder.LABL', 'com.apple.finder.ACTN', 'NSToolbarSpaceItem', 'com.apple.finder.SRCH'];

function run(argv) {
  const action = argv[0];
  const target = argv[1].replace(/\/+$/, '');
  const domain = argv[2] || 'com.apple.finder';
  const defaults = $.NSUserDefaults.alloc.initWithSuiteName(domain);
  const stored = defaults.objectForKey(KEY);
  const config = stored.isNil() ? $.NSMutableDictionary.dictionary : stored.mutableCopy;
  const read = (name) => {
    const value = config.objectForKey(name);
    return value.isNil() ? null : value;
  };
  const storedIds = read('TB Item Identifiers') || read('TB Default Item Identifiers');
  const ids = storedIds ? ObjC.deepUnwrap(storedIds) : FALLBACK.slice();
  // 位置 → 原样的字典（ObjC 对象，里面的 alias 数据不经过 JS）
  const plists = {};
  const storedPlists = read('TB Item Plists');
  if (storedPlists) {
    const keys = storedPlists.allKeys;
    for (let i = 0; i < keys.count; i++) {
      const key = keys.objectAtIndex(i).js;
      plists[Number(key)] = storedPlists.objectForKey(key);
    }
  }
  // 按钮现在指着的路径：访达存的是文件引用（file:///.file/id=…），换成路径再比
  const pathOf = (item) => {
    const text = item.objectForKey('_CFURLString');
    if (text.isNil()) return null;
    const url = $.NSURL.URLWithString(text);
    if (url.isNil()) return null;
    const file = url.filePathURL;
    return file.isNil() ? null : file.path.js.replace(/\/+$/, '');
  };
  // SSHL 被整个换掉（更新、重新安装）后文件引用指着已经没了的那份，访达下次显示时才顺着 alias 里记的路径找回来：
  // 这期间按 alias 里记的名字认
  const name = target.split('/').pop();
  const aliasText = (item) => {
    const data = item.objectForKey('_CFURLAliasData');
    if (data.isNil()) return '';
    const text = $.NSString.alloc.initWithDataEncoding(data, $.NSISOLatin1StringEncoding);
    return text.isNil() ? '' : text.js;
  };
  const isOurs = (item) => {
    const path = pathOf(item);
    return path === target || (path === null && aliasText(item).includes(name));
  };
  const ours = Object.keys(plists).map(Number).filter((i) => ids[i] === LOC && isOurs(plists[i]));

  if (action === 'has') return ours.length > 0 ? 'yes' : 'no';
  if (action === 'dump') {
    const paths = {};
    for (const i of Object.keys(plists)) paths[i] = pathOf(plists[i]);
    return JSON.stringify({ ids, paths });
  }

  let changed = {};
  for (const [i, item] of Object.entries(plists)) changed[Number(i)] = item;
  // 去掉一项：后面的位置都往前挪一格
  const removeAt = (at) => {
    ids.splice(at, 1);
    const next = {};
    for (const [i, item] of Object.entries(changed)) {
      const n = Number(i);
      if (n !== at) next[n > at ? n - 1 : n] = item;
    }
    changed = next;
  };
  if (action === 'remove') {
    if (ours.length === 0) return 'ok';
    ours.sort((a, b) => b - a).forEach(removeAt);
  } else if (action === 'add') {
    if (ours.length > 0) return 'ok';
    // 放在已有的自定义按钮后面（同类的挨着），没有就放在前进 / 后退后面
    const lastLoc = ids.lastIndexOf(LOC);
    const back = ids.indexOf('com.apple.finder.BACK');
    const at = lastLoc >= 0 ? lastLoc + 1 : back + 1;
    ids.splice(at, 0, LOC);
    const next = {};
    for (const [i, item] of Object.entries(changed)) {
      const n = Number(i);
      next[n >= at ? n + 1 : n] = item;
    }
    const item = $.NSMutableDictionary.dictionary;
    item.setObjectForKey($.NSURL.fileURLWithPathIsDirectory(target, true).absoluteString, '_CFURLString');
    item.setObjectForKey($(15), '_CFURLStringType');
    next[at] = item;
    changed = next;
  } else {
    throw new Error('unknown action ' + action);
  }

  const out = $.NSMutableDictionary.dictionary;
  for (const [i, item] of Object.entries(changed)) out.setObjectForKey(item, String(i));
  config.setObjectForKey($(ids), 'TB Item Identifiers');
  config.setObjectForKey(out, 'TB Item Plists');
  defaults.setObjectForKey(config, KEY);
  defaults.synchronize;
  return 'ok';
}

/**
 * 拖拽落点这类只含原始值字段的小对象：字段逐个相同就算同一个。指针每次移动都会算出一个新对象，
 * 落点没变时不该 setState，否则整个列表按指针频率重渲染。
 */
export function shallowEqual<T extends object>(a: T | null, b: T | null) {
  if (a === b) return true;
  if (a === null || b === null) return false;
  const keys = Object.keys(a) as (keyof T)[];
  return keys.length === Object.keys(b).length && keys.every(key => Object.hasOwn(b, key) && Object.is(a[key], b[key]));
}

// 分类切换会卸载页面组件,组件里的 useState 随之丢失。视图状态(筛选、图层、展开集合、
// 选中项)因此存在组件之外,按 key 保存;webview 重建时自然清零。
const store = new Map<string, unknown>();

export function readViewState<T>(key: string, init: T): T {
  return store.has(key) ? (store.get(key) as T) : init;
}

export function writeViewState<T>(key: string, value: T): void {
  store.set(key, value);
}

export function resetViewState(): void {
  store.clear();
}

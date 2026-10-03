import type { ViewPayload } from '../types.ts';

export function App({ payload }: { payload: ViewPayload }) {
  if (!payload.loaded) return <div class="gc-empty">加载中…</div>;
  if (payload.groups.length === 0) return <div class="gc-empty">还没有群聊</div>;
  return <div class="gc-root">{payload.groups.length} 个群聊</div>;
}

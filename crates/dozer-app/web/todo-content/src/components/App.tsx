import type { ViewPayload } from '../types.ts';

export function App({ payload }: { payload: ViewPayload }) {
  return <div class="todo-root">{payload.items.length} 项</div>;
}

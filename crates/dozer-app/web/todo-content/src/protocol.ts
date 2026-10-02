import type { ViewPayload } from './types.ts';

export interface PushState {
  revision: number;
  payload: ViewPayload | null;
}

/** 应用一条 Rust 推送。revision 小于已应用值的推送(快速切换时晚到的旧响应)
 *  与无法解析/缺 payload 的推送一律忽略,状态原样返回。 */
export function applyEnvelope(prev: PushState, json: string): PushState {
  try {
    const env = JSON.parse(json) as { revision?: number; payload?: ViewPayload };
    if (!env.payload || typeof env.revision !== 'number') return prev;
    if (env.revision < prev.revision) return prev;
    return { revision: env.revision, payload: env.payload };
  } catch {
    return prev;
  }
}

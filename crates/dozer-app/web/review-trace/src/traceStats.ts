import type { AiTurnData } from './types.ts';

// 1000 进 "k",一位小数、去掉多余的 ".0"——轨迹摘要行放不下完整数字,
// 只需要一眼看出量级。
export function formatTokenCount(n: number): string {
  if (n >= 1000) {
    return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
  }
  return String(n);
}

// 轨迹折叠行(未展开时)就能看到的统计摘要:工具调用次数(+失败数)、
// token 总量(in+out+cache_read+cache_write 相加,不细分——细分对一行
// 摘要来说信息过载,想看明细本来就有下面的工具结果/thinking 展开)。
// 两项都没有(纯 thinking、且老协议帧没有 token 字段)时返回空串,调用方
// 据此决定要不要挂这个 span。
export function buildTraceStatsLabel(v: AiTurnData): string {
  const parts: string[] = [];
  const toolCount = (v.tool_calls || []).length;
  if (toolCount > 0) {
    const errCount = (v.tool_results || []).filter((r) => r.is_error).length;
    parts.push(`${toolCount} 次工具调用${errCount > 0 ? `(${errCount} 失败)` : ''}`);
  }
  const totalTokens =
    (v.tokens_in || 0) + (v.tokens_out || 0) + (v.tokens_cache_read || 0) + (v.tokens_cache_write || 0);
  if (totalTokens > 0) {
    parts.push(`${formatTokenCount(totalTokens)} tokens`);
  }
  return parts.join(' · ');
}

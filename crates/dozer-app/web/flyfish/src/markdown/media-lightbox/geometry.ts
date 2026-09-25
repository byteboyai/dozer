// Markdown 媒体灯箱的纯几何逻辑。无 DOM、无副作用、无全局状态,只做
// 适应比例、缩放锚点、平移 clamp 与 transform 字符串生成,便于表驱动单测。
//
// 坐标约定:
// - 内容坐标(content)= 媒体的固有像素空间,原点在媒体左上角。
// - 视口坐标(viewport)= stage 可见区域,原点在 stage 左上角。
// - transform 施加顺序为 scale 后 translate(等价于 CSS `scale() translate()`
//   的矩阵语义),见 stageTransform。

import {
  DEFAULT_FIT_INSETS,
  SCALE_FIT_MAX,
  SCALE_MAX,
  SCALE_MIN,
  type FitInsets,
  type Size,
  type ViewportState,
  type ZoomAnchor,
} from './types.ts';

export class GeometryError extends Error {}

const assertFinite = (value: number, what: string): number => {
  if (!Number.isFinite(value)) {
    throw new GeometryError(`${what} must be finite, got ${value}`);
  }
  return value;
};

const assertFiniteSize = (size: Size, what: string): Size => {
  assertFinite(size.width, `${what}.width`);
  assertFinite(size.height, `${what}.height`);
  if (size.width < 0 || size.height < 0) {
    throw new GeometryError(`${what} must be non-negative, got ${JSON.stringify(size)}`);
  }
  return size;
};

const assertFinitePoint = (
  point: { x: number; y: number },
  what: string
): { x: number; y: number } => {
  assertFinite(point.x, `${what}.x`);
  assertFinite(point.y, `${what}.y`);
  return point;
};

const clamp = (value: number, min: number, max: number) => {
  if (min > max) return min;
  return Math.min(max, Math.max(min, value));
};

/**
 * 计算 contain 适应比例。
 * - 目标:完整看到内容,因此取较紧的一维。
 * - 上限 100%(不主动放大小于视口的位图),下限 10%。
 * - 在扣除 insets 后的可用视口内 contain。
 * - 任一边不可用(0/非有限/内容未知)时不返回 NaN/Infinity:返回安全值。
 */
export const fitScale = (
  content: Size,
  viewport: Size,
  insets: FitInsets = DEFAULT_FIT_INSETS
): number => {
  assertFiniteSize(content, 'content');
  assertFiniteSize(viewport, 'viewport');

  const availW = viewport.width - insets.left - insets.right;
  const availH = viewport.height - insets.top - insets.bottom;

  // 视口不可用(还没测量到)或内容零尺寸 → 退回 100%(而非 0/NaN)。
  if (availW <= 0 || availH <= 0) {
    return 1;
  }
  if (content.width <= 0 || content.height <= 0) {
    return SCALE_FIT_MAX;
  }

  const sx = availW / content.width;
  const sy = availH / content.height;
  const raw = Math.min(sx, sy, SCALE_FIT_MAX);
  return clamp(raw, SCALE_MIN, SCALE_MAX);
};

/** 把视口坐标点换算成内容坐标点(给定当前 scale 与 translation)。 */
export const contentPointAt = (
  viewportPoint: { x: number; y: number },
  state: Pick<ViewportState, 'scale' | 'translation'>
): { x: number; y: number } => {
  const scale = assertFinite(state.scale, 'scale');
  if (scale === 0) {
    throw new GeometryError('scale must not be 0');
  }
  return {
    x: (viewportPoint.x - state.translation.x) / scale,
    y: (viewportPoint.y - state.translation.y) / scale,
  };
};

/**
 * 以指针为锚点缩放:缩放前后指针下的内容坐标保持不变。
 * 返回新的 scale 与 translation(clamp 到 10%~500%)。
 * 锚点的 contentPoint 若未提供则由当前状态推导。
 */
export const zoomAtPoint = (
  state: ViewportState,
  nextScale: number,
  viewportPoint: { x: number; y: number },
  anchor?: ZoomAnchor
): ViewportState => {
  assertFinitePoint(viewportPoint, 'viewportPoint');
  const prevScale = assertFinite(state.scale, 'scale');
  if (prevScale === 0) {
    throw new GeometryError('scale must not be 0');
  }

  const scale = clamp(assertFinite(nextScale, 'nextScale'), SCALE_MIN, SCALE_MAX);

  const content = anchor?.contentPoint ?? contentPointAt(viewportPoint, state);
  const translation = {
    x: viewportPoint.x - content.x * scale,
    y: viewportPoint.y - content.y * scale,
  };

  return { scale, translation, mode: 'manual' };
};

/**
 * 平移 clamp:
 * - 内容(已缩放)小于视口时,沿该轴居中。
 * - 内容大于视口时,至少保留可恢复边缘(不允许内容永久拖出视口)。
 * - 任何输入都返回有限值。
 */
export const clampTranslation = (
  translation: { x: number; y: number },
  content: Size,
  viewport: Size,
  scale: number
): { x: number; y: number } => {
  assertFiniteSize(content, 'content');
  assertFiniteSize(viewport, 'viewport');
  const s = clamp(assertFinite(scale, 'scale'), SCALE_MIN, SCALE_MAX);

  const scaledW = content.width * s;
  const scaledH = content.height * s;

  const clampAxis = (value: number, scaled: number, viewportSpan: number) => {
    assertFinite(value, 'translation');
    if (scaled <= viewportSpan) {
      // 内容比视口小:居中。
      return (viewportSpan - scaled) / 2;
    }
    // 内容比视口大:左/上边界不超过 0,右/下边界不小于视口 - 内容。
    const min = viewportSpan - scaled;
    const max = 0;
    return clamp(value, min, max);
  };

  return {
    x: clampAxis(translation.x, scaledW, viewport.width),
    y: clampAxis(translation.y, scaledH, viewport.height),
  };
};

/** 在给定视口尺寸变化后重新 clamp 状态;fit 模式重新 fit,manual 保留 scale。 */
export const reconcileOnResize = (
  state: ViewportState,
  content: Size,
  viewport: Size,
  insets: FitInsets = DEFAULT_FIT_INSETS
): ViewportState => {
  const scale = state.mode === 'fit' ? fitScale(content, viewport, insets) : state.scale;
  const translation = clampTranslation(state.translation, content, viewport, scale);
  return { scale, translation, mode: state.mode };
};

/** 居中一个给定 scale 的内容(等价于 translation 归零后的 clamp)。 */
export const centeredState = (
  scale: number,
  content: Size,
  viewport: Size,
  mode: ViewportState['mode'] = 'manual'
): ViewportState => {
  const s = clamp(assertFinite(scale, 'scale'), SCALE_MIN, SCALE_MAX);
  return {
    scale: s,
    translation: clampTranslation({ x: 0, y: 0 }, content, viewport, s),
    mode,
  };
};

/**
 * 生成 stage 的 CSS transform。只作用于单一 stage 元素,使用 translate3d +
 * scale 提升到合成层。translate 以内容左上角为参考,这里把内容先平移到
 * translation 再缩放。
 */
export const stageTransform = (state: Pick<ViewportState, 'scale' | 'translation'>): string => {
  const s = assertFinite(state.scale, 'scale');
  const { x, y } = state.translation;
  assertFinite(x, 'translation.x');
  assertFinite(y, 'translation.y');
  return `translate3d(${x}px, ${y}px, 0) scale(${s})`;
};

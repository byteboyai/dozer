// Markdown 媒体灯箱的共享类型。这里的几何/状态是**纯数据**,不依赖 DOM,
// 由 geometry.ts 的函数消费,方便表驱动单测。

export type MediaKind = 'image' | 'mermaid';

/** 灯箱操作的目标媒体。 */
export interface MediaDescriptor {
  kind: MediaKind;
  /** 可访问名称,用于 dialog/aria-label;从 alt / aria-label / 标题派生。 */
  accessibleName: string;
  /** 源节点:普通图片是 <img>,Mermaid 是净化后的 <svg>。 */
  source: HTMLElement;
  /**
   * 固有尺寸(未缩放像素)。位图在 naturalWidth/naturalHeight 就绪前为
   * null;调用方不得把未知当成 0。
   */
  intrinsicWidth: number | null;
  intrinsicHeight: number | null;
}

/** 视口(stage 可见区域)尺寸,单位 CSS 像素。 */
export interface Size {
  width: number;
  height: number;
}

/** 平移量,单位 CSS 像素。 */
export interface Translation {
  x: number;
  y: number;
}

/**
 * 灯箱的缩放/平移状态。
 * - scale: 1 = 100%,范围由 SCALE_MIN/SCALE_MAX 约束。
 * - translation: 内容在视口内的位移(施加顺序:先平移后缩放,见 stageCSS)。
 * - mode: 'fit' 表示当前跟随可用视口自适应,resize 时重新 fit;
 *         'manual' 表示用户已手动缩放,resize 时保留 scale。
 */
export interface ViewportState {
  scale: number;
  translation: Translation;
  mode: 'fit' | 'manual';
}

/** 缩放锚点:相对视口的指针坐标 + 该点下的内容坐标(已归一化到内容像素)。 */
export interface ZoomAnchor {
  /** 指针相对视口左上角的坐标。 */
  viewportPoint: { x: number; y: number };
  /** 指针下的内容坐标(未缩放、未平移的媒体像素空间)。 */
  contentPoint: { x: number; y: number };
}

export const SCALE_MIN = 0.1;
export const SCALE_MAX = 5;
export const SCALE_FIT_MAX = 1;

/** 工具栏与安全边距:fit 时从可用视口里扣除,避免内容贴边被工具挡。 */
export interface FitInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export const DEFAULT_FIT_INSETS: FitInsets = {
  top: 0,
  right: 0,
  bottom: 0,
  left: 0,
};

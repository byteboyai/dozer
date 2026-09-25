// Markdown 媒体灯箱 DOM 控制器。
//
// 单实例、可销毁、无全局泄漏:每个 renderer 实例在 `article` 上安装一份,
// `unmount()` 时 destroy()。灯箱只作用于自己所在的 Flyfish 渲染 surface,
// 不触碰 host.html、Rust 注入或 iced。
//
// 集成:renderMarkdown 在 HTML + Mermaid 就绪后调用
// `installMarkdownMediaLightbox({ viewer, article, theme })`,拿到 `destroy()`。

import {
  clampTranslation,
  contentPointAt,
  fitScale,
  reconcileOnResize,
  stageTransform,
  zoomAtPoint,
} from './geometry.ts';
import { markdownMediaLightboxStyle, LIGHTBOX_STYLE_ID } from './styles.ts';
import { rewriteSvgIds } from './svgIds.ts';
import {
  SCALE_FIT_MAX,
  SCALE_MAX,
  SCALE_MIN,
  type FitInsets,
  type MediaDescriptor,
  type ViewportState,
} from './types.ts';

export interface InstallOptions {
  /** 滚动容器(.markdown-viewer),用于打开时锁滚动、关闭时恢复。 */
  viewer: HTMLElement;
  /** 已渲染完毕的 Markdown 正文容器。 */
  article: HTMLElement;
  theme?: 'light' | 'dark' | 'system';
  /** fit 时扣除的工具栏/安全边距,默认给顶部工具栏留白由调用方注入。 */
  fitInsets?: FitInsets;
}

export interface LightboxHandle {
  destroy: () => void;
  /** 测试/调试用:当前是否打开。 */
  isOpen: () => boolean;
  /** 测试/调试用:当前绑定并可打开的媒体数量。 */
  mediaCount: () => number;
}

const ZOOM_STEP = 1.25;
const WHEEL_ZOOM_STEP = 1.1;
const KEY_ZOOM_STEP = 1.25;
const LIGHTBOX_FIT_INSETS: FitInsets = { top: 72, right: 24, bottom: 24, left: 24 };

const ICONS = {
  close:
    '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06Z"/></svg>',
  zoomOut:
    '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M2.75 8a.75.75 0 0 1 .75-.75h9a.75.75 0 0 1 0 1.5h-9A.75.75 0 0 1 2.75 8Z"/></svg>',
  zoomIn:
    '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M8 2.75a.75.75 0 0 1 .75.75v3.75h3.75a.75.75 0 0 1 0 1.5H8.75v3.75a.75.75 0 0 1-1.5 0V8.75H3.5a.75.75 0 0 1 0-1.5h3.75V3.5A.75.75 0 0 1 8 2.75Z"/></svg>',
  fit:
    '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M2.75 3.5c0-.41.34-.75.75-.75h3a.75.75 0 0 1 0 1.5H4.25V6a.75.75 0 0 1-1.5 0V3.5Zm7.75-.75h3c.41 0 .75.34.75.75V6a.75.75 0 0 1-1.5 0V4.25H10.5a.75.75 0 0 1 0-1.5Zm-8.25 9h1.5V10a.75.75 0 0 1 1.5 0v3c0 .41-.34.75-.75.75h-3a.75.75 0 0 1 0-1.5Zm9.75 0h-1.5V10a.75.75 0 0 1 1.5 0v3c0 .41-.34.75-.75.75h-3a.75.75 0 0 1 0-1.5Z"/></svg>',
};

const isEditableTarget = (target: EventTarget | null): boolean => {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== 'string') return false;
  const tag = el.tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return true;
  return el.isContentEditable === true;
};

const isSvgRoot = (el: Element): boolean =>
  el.namespaceURI === 'http://www.w3.org/2000/svg' && el.localName === 'svg';

const accessibleNameFor = (el: Element, fallback: string): string => {
  const label =
    el.getAttribute('aria-label') ||
    el.getAttribute('alt') ||
    el.getAttribute('title') ||
    '';
  const trimmed = label.trim();
  return trimmed.length > 0 ? trimmed : fallback;
};

/** 从 <img> / Mermaid SVG 收集可打开的媒体。 */
export const collectMedia = (article: HTMLElement, documentRef: Document): MediaDescriptor[] => {
  const media: MediaDescriptor[] = [];

  const images = article.querySelectorAll<HTMLImageElement>(
    'img:not([data-dozer-lightbox-skip])'
  );
  for (const img of images) {
    // 仅匹配成功加载的图片;不把 0 尺寸/未加载当 0% 处理。
    const loaded = img.complete && img.naturalWidth > 0;
    if (!loaded) continue;
    media.push({
      kind: 'image',
      accessibleName: accessibleNameFor(img, 'Image'),
      source: img,
      intrinsicWidth: img.naturalWidth,
      intrinsicHeight: img.naturalHeight,
    });
  }

  const mermaidSvgs = article.querySelectorAll<SVGSVGElement>(
    '.markdown-mermaid[data-markdown-mermaid="rendered"] > svg'
  );
  for (const svg of mermaidSvgs) {
    if (svg.ownerDocument !== documentRef) continue;
    media.push({
      kind: 'mermaid',
      accessibleName: accessibleNameFor(svg, 'Mermaid diagram'),
      source: svg,
      intrinsicWidth: null,
      intrinsicHeight: null,
    });
  }

  return media;
};

export const installMarkdownMediaLightbox = (options: InstallOptions): LightboxHandle => {
  const { viewer, article } = options;
  const documentRef = article.ownerDocument;
  const windowRef = documentRef.defaultView;
  const fitInsets = options.fitInsets ?? LIGHTBOX_FIT_INSETS;

  if (!documentRef.getElementById(LIGHTBOX_STYLE_ID)) {
    const style = documentRef.createElement('style');
    style.id = LIGHTBOX_STYLE_ID;
    style.textContent = markdownMediaLightboxStyle;
    (documentRef.head ?? documentRef.documentElement).appendChild(style);
  }

  const overlay = documentRef.createElement('div');
  overlay.className = 'dozer-media-lightbox';
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-label', 'Media viewer');
  overlay.hidden = true;

  const wrap = documentRef.createElement('div');
  wrap.className = 'dozer-media-lightbox__stage-wrap';
  const stage = documentRef.createElement('div');
  stage.className = 'dozer-media-lightbox__stage';
  wrap.appendChild(stage);

  const toolbar = documentRef.createElement('div');
  toolbar.className = 'dozer-media-lightbox__toolbar';
  toolbar.setAttribute('role', 'toolbar');

  const makeButton = (cls: string, label: string, icon: string) => {
    const btn = documentRef.createElement('button');
    btn.type = 'button';
    btn.className = `dozer-media-lightbox__btn ${cls}`;
    btn.setAttribute('aria-label', label);
    btn.title = label;
    btn.innerHTML = icon;
    return btn;
  };

  const closeBtn = makeButton('dozer-media-lightbox__btn--close', 'Close', ICONS.close);
  const zoomOutBtn = makeButton('', 'Zoom out', ICONS.zoomOut);
  const zoomInBtn = makeButton('', 'Zoom in', ICONS.zoomIn);
  const fitBtn = makeButton('', 'Fit to window', ICONS.fit);
  const resetBtn = documentRef.createElement('button');
  resetBtn.type = 'button';
  resetBtn.className = 'dozer-media-lightbox__btn';
  resetBtn.setAttribute('aria-label', 'Actual size 100%');
  resetBtn.title = 'Actual size 100%';
  resetBtn.textContent = '100%';

  const zoomLabel = documentRef.createElement('span');
  zoomLabel.className = 'dozer-media-lightbox__zoom';
  zoomLabel.setAttribute('aria-live', 'polite');

  toolbar.append(zoomOutBtn, zoomLabel, zoomInBtn, resetBtn, fitBtn, closeBtn);
  overlay.append(toolbar, wrap);

  // 状态
  let active = false;
  let stageMedia: Element | null = null;
  let contentSize: { width: number; height: number } = { width: 0, height: 0 };
  let state: ViewportState = { scale: 1, translation: { x: 0, y: 0 }, mode: 'fit' };
  let svgSequence = 0;
  let destroyed = false;

  // 关闭时恢复
  let savedActiveElement: Element | null = null;
  let savedScrollTop = 0;
  let savedScrollLeft = 0;
  let savedOverflow = '';
  let savedFocusMedia: MediaDescriptor | null = null;

  // 拖拽
  let dragPointerId: number | null = null;
  let dragStart = { x: 0, y: 0 };
  let dragStartTranslation = { x: 0, y: 0 };

  let resizeObserver: ResizeObserver | null = null;

  const viewportSize = () => {
    const rect = wrap.getBoundingClientRect();
    return { width: rect.width, height: rect.height };
  };

  const applyState = () => {
    stage.style.transform = stageTransform(state);
    stage.classList.toggle('is-zoomed', state.scale > SCALE_FIT_MAX + 1e-6);
    zoomLabel.textContent = `${Math.round(state.scale * 100)}%`;
    zoomOutBtn.disabled = state.scale <= SCALE_MIN + 1e-6;
    zoomInBtn.disabled = state.scale >= SCALE_MAX - 1e-6;
  };

  const withClamp = (next: ViewportState): ViewportState => ({
    ...next,
    translation: clampTranslation(next.translation, contentSize, viewportSize(), next.scale),
  });

  const setScaleCentered = (scale: number, mode: ViewportState['mode']) => {
    const viewport = viewportSize();
    const clamped = Math.min(SCALE_MAX, Math.max(SCALE_MIN, scale));
    // 以视口中心为锚点。
    const anchorPoint = { x: viewport.width / 2, y: viewport.height / 2 };
    const anchor = {
      viewportPoint: anchorPoint,
      contentPoint: contentPointAt(anchorPoint, state),
    };
    state = withClamp(zoomAtPoint(state, clamped, anchorPoint, anchor));
    if (mode === 'fit') {
      state = { ...state, mode };
    }
    applyState();
  };

  const fitToViewport = () => {
    const viewport = viewportSize();
    const scale = fitScale(contentSize, viewport, fitInsets);
    state = {
      scale,
      translation: clampTranslation({ x: 0, y: 0 }, contentSize, viewport, scale),
      mode: 'fit',
    };
    applyState();
  };

  const measureStageMedia = (el: Element, descriptor: MediaDescriptor) => {
    if (descriptor.intrinsicWidth && descriptor.intrinsicHeight) {
      return { width: descriptor.intrinsicWidth, height: descriptor.intrinsicHeight };
    }
    // Mermaid SVG:读取 viewBox 或渲染后的 bbox。
    const svg = el as unknown as SVGSVGElement;
    if (typeof svg.viewBox?.baseVal?.width === 'number' && svg.viewBox.baseVal.width > 0) {
      return { width: svg.viewBox.baseVal.width, height: svg.viewBox.baseVal.height };
    }
    const rect = el.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      return { width: rect.width, height: rect.height };
    }
    return { width: 1, height: 1 };
  };

  const buildStageMedia = (descriptor: MediaDescriptor): Element => {
    if (descriptor.kind === 'image') {
      const source = descriptor.source as HTMLImageElement;
      const img = documentRef.createElement('img');
      img.className = 'dozer-media-lightbox__media dozer-media-lightbox__media--image';
      img.alt = '';
      img.draggable = false;
      // 复用已解析 URL;不在灯箱里重新请求网络。
      img.src = source.currentSrc || source.src;
      return img;
    }
    // Mermaid:深克隆已净化的 SVG,并重写内部 ID 避免与正文 DOM 冲突。
    const clone = descriptor.source.cloneNode(true) as SVGSVGElement;
    clone.classList.add('dozer-media-lightbox__media');
    clone.removeAttribute('width');
    clone.removeAttribute('height');
    svgSequence += 1;
    rewriteSvgIds(clone, `dozer-lb-${Date.now().toString(36)}-${svgSequence}-`);
    return clone;
  };

  const open = (descriptor: MediaDescriptor, opener: Element | null) => {
    if (active || destroyed) return;
    active = true;
    savedActiveElement = opener ?? (documentRef.activeElement as Element | null);
    savedFocusMedia = descriptor;

    // 记录正文滚动与整页 zoom 相关的状态(整页 zoom 在本控制器内只读)。
    savedScrollTop = viewer.scrollTop;
    savedScrollLeft = viewer.scrollLeft;
    savedOverflow = viewer.style.overflow;
    viewer.style.overflow = 'hidden';

    if (stageMedia) {
      stageMedia.remove();
      stageMedia = null;
    }
    stageMedia = buildStageMedia(descriptor);
    stage.replaceChildren(stageMedia);

    overlay.hidden = false;
    documentRef.body.appendChild(overlay);

    contentSize = measureStageMedia(stageMedia, descriptor);
    stage.style.width = `${contentSize.width}px`;
    stage.style.height = `${contentSize.height}px`;
    overlay.setAttribute('aria-label', `Media viewer: ${descriptor.accessibleName}`);
    state = { scale: 1, translation: { x: 0, y: 0 }, mode: 'fit' };
    fitToViewport();

    // 尺寸可能因图片解码而稍后变化;ResizeObserver 负责 re-fit。
    if (windowRef && typeof windowRef.ResizeObserver === 'function') {
      resizeObserver = new windowRef.ResizeObserver(onResize);
      resizeObserver.observe(wrap);
      resizeObserver.observe(stageMedia);
    }

    documentRef.addEventListener('keydown', onKeyDown, true);
    windowRef?.addEventListener('blur', onWindowBlur);
    closeBtn.focus();
  };

  const restore = () => {
    viewer.style.overflow = savedOverflow;
    viewer.scrollTop = savedScrollTop;
    viewer.scrollLeft = savedScrollLeft;
  };

  const close = () => {
    if (!active) return;
    active = false;

    if (dragPointerId !== null) {
      try {
        stage.releasePointerCapture?.(dragPointerId);
      } catch {
        // ignore
      }
      dragPointerId = null;
    }

    documentRef.removeEventListener('keydown', onKeyDown, true);
    windowRef?.removeEventListener('blur', onWindowBlur);
    resizeObserver?.disconnect();
    resizeObserver = null;

    overlay.hidden = true;
    stage.replaceChildren();
    stageMedia = null;
    overlay.remove();

    restore();

    // 焦点恢复到打开前的元素;若来源媒体已从 DOM 移除则退回到其可聚焦代理。
    const target =
      (savedActiveElement && documentRef.contains(savedActiveElement)
        ? savedActiveElement
        : null) ??
      (savedFocusMedia && documentRef.contains(savedFocusMedia.source)
        ? savedFocusMedia.source
        : null);
    if (target && typeof (target as HTMLElement).focus === 'function') {
      (target as HTMLElement).focus({ preventScroll: true });
    }
    savedActiveElement = null;
    savedFocusMedia = null;
  };

  const onResize = () => {
    if (!active) return;
    const viewport = viewportSize();
    const next = reconcileOnResize(state, contentSize, viewport, fitInsets);
    state = withClamp(next);
    applyState();
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (!active) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (isEditableTarget(event.target)) return;

    if (event.key === 'Tab') {
      const focusable = Array.from(
        toolbar.querySelectorAll<HTMLButtonElement>('button:not([disabled])')
      );
      if (focusable.length === 0) return;
      const current = focusable.indexOf(documentRef.activeElement as HTMLButtonElement);
      const next = event.shiftKey
        ? (current <= 0 ? focusable.length - 1 : current - 1)
        : (current < 0 || current === focusable.length - 1 ? 0 : current + 1);
      event.preventDefault();
      focusable[next].focus();
      return;
    }

    const key = event.key;
    if (key === '+' || key === '=') {
      event.preventDefault();
      setScaleCentered(state.scale * KEY_ZOOM_STEP, 'manual');
    } else if (key === '-' || key === '_') {
      event.preventDefault();
      setScaleCentered(state.scale / KEY_ZOOM_STEP, 'manual');
    } else if (key === '0') {
      event.preventDefault();
      setScaleCentered(1, 'manual');
    } else if (key === 'f' || key === 'F') {
      event.preventDefault();
      fitToViewport();
    }
  };

  const onWindowBlur = () => {
    // 失焦时结束拖拽,避免 capture 丢失后状态卡住。
    endDrag();
  };

  const wheelPoint = (event: WheelEvent) => {
    const rect = wrap.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  const onWheel = (event: WheelEvent) => {
    if (!active) return;
    if (!(event.metaKey || event.ctrlKey)) {
      // 普通滚轮平移画布；边界由几何层统一 clamp。
      state = withClamp({
        ...state,
        translation: {
          x: state.translation.x - event.deltaX,
          y: state.translation.y - event.deltaY,
        },
        mode: 'manual',
      });
      event.preventDefault();
      applyState();
      return;
    }
    event.preventDefault();
    const point = wheelPoint(event);
    const factor = event.deltaY < 0 ? WHEEL_ZOOM_STEP : 1 / WHEEL_ZOOM_STEP;
    const anchor = { viewportPoint: point, contentPoint: contentPointAt(point, state) };
    state = withClamp(zoomAtPoint(state, state.scale * factor, point, anchor));
    applyState();
  };

  const onPointerDown = (event: PointerEvent) => {
    if (!active) return;
    // 只响应主键/笔;多指场景:已有拖拽时忽略第二指针,不破坏状态。
    if (dragPointerId !== null) return;
    if (event.pointerType === 'mouse' && event.button !== 0) return;
    dragPointerId = event.pointerId;
    dragStart = { x: event.clientX, y: event.clientY };
    dragStartTranslation = { ...state.translation };
    stage.classList.add('is-dragging');
    try {
      stage.setPointerCapture(event.pointerId);
    } catch {
      // ignore
    }
  };

  const onPointerMove = (event: PointerEvent) => {
    if (dragPointerId === null || event.pointerId !== dragPointerId) return;
    const dx = event.clientX - dragStart.x;
    const dy = event.clientY - dragStart.y;
    state = withClamp({
      ...state,
      translation: {
        x: dragStartTranslation.x + dx,
        y: dragStartTranslation.y + dy,
      },
      mode: 'manual',
    });
    applyState();
    event.preventDefault();
  };

  const endDrag = () => {
    if (dragPointerId !== null) {
      try {
        stage.releasePointerCapture?.(dragPointerId);
      } catch {
        // ignore
      }
    }
    dragPointerId = null;
    stage.classList.remove('is-dragging');
  };

  const onPointerUp = (event: PointerEvent) => {
    if (dragPointerId === null || event.pointerId !== dragPointerId) return;
    endDrag();
  };

  const onOverlayPointerDown = (event: PointerEvent) => {
    // 背景点击关闭只认 target === overlay;媒体/工具栏/画布拖拽不误关。
    if (event.target === overlay && event.pointerType !== 'touch') {
      close();
    }
  };

  const onDoubleClick = (event: MouseEvent) => {
    if (!active || !(event.target instanceof windowRef!.Element)) return;
    if (!stage.contains(event.target)) return;
    event.preventDefault();
    if (state.mode === 'fit') {
      setScaleCentered(1, 'manual');
    } else {
      fitToViewport();
    }
  };

  // 打开媒体:绑定点击与键盘 Enter/Space。
  const openerListeners: Array<{
    target: EventTarget;
    type: 'click' | 'keydown';
    listener: EventListener;
  }> = [];
  const pendingImages: Array<{
    img: HTMLImageElement;
    load: EventListener;
    error: EventListener;
  }> = [];

  const descriptorByElement = new Map<Element, MediaDescriptor>();

  const resolveDescriptor = (el: Element): MediaDescriptor | null => {
    const direct = descriptorByElement.get(el);
    if (direct) return direct;
    // 链接包裹图片:点击 img 或点击外层 <a> 的图片区域都算。
    const img = (el as HTMLElement).closest?.('img');
    if (img) {
      return descriptorByElement.get(img) ?? null;
    }
    return null;
  };

  const bindOpener = (el: Element, descriptor: MediaDescriptor) => {
    el.setAttribute('data-dozer-lightbox-bound', 'true');
    // 图片位于 <a> 内时,不为 img 额外加 tabindex,避免嵌套交互元素的非法语义;
    // 复用外层链接的焦点。其余情况让媒体本身可聚焦。
    const link = (el as HTMLElement).closest?.('a[href]') ?? null;
    const insideLink = Boolean(link);
    if (!insideLink && !el.hasAttribute('tabindex')) {
      el.setAttribute('tabindex', '0');
    }
    if (isSvgRoot(el) && !el.hasAttribute('role')) {
      el.setAttribute('role', 'img');
    }
    const handler = (event: Event) => {
      const media = resolveDescriptor(el);
      if (!media) return;
      // 链接包裹图片时，点图片打开灯箱，不继续执行链接导航。
      event.preventDefault();
      event.stopPropagation();
      open(media, el);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      event.preventDefault();
      const media = resolveDescriptor(el);
      if (!media) return;
      open(media, el);
    };
    el.addEventListener('click', handler);
    openerListeners.push({ target: el, type: 'click', listener: handler });
    const keyboardTarget = link ?? el;
    keyboardTarget.addEventListener('keydown', key as EventListener);
    openerListeners.push({ target: keyboardTarget, type: 'keydown', listener: key as EventListener });
  };

  const clearBindings = () => {
    for (const { target, type, listener } of openerListeners) {
      target.removeEventListener(type, listener);
    }
    openerListeners.length = 0;
    for (const { img, load, error } of pendingImages) {
      img.removeEventListener('load', load);
      img.removeEventListener('error', error);
    }
    pendingImages.length = 0;
    descriptorByElement.clear();
  };

  const refreshBindings = () => {
    clearBindings();

    const media = collectMedia(article, documentRef);
    for (const descriptor of media) {
      descriptorByElement.set(descriptor.source, descriptor);
      bindOpener(descriptor.source, descriptor);
    }
    currentMediaCount = media.length;

    // 图片 load 不会触发 child-list mutation；为安装时尚未解码的图片显式补绑定。
    for (const img of article.querySelectorAll<HTMLImageElement>(
      'img:not([data-dozer-lightbox-skip])'
    )) {
      // complete=true + naturalWidth=0 是永久失败，不再等待不存在的第二次 error。
      if (img.complete) continue;
      const load: EventListener = () => refreshBindings();
      const error: EventListener = () => refreshBindings();
      img.addEventListener('load', load, { once: true });
      img.addEventListener('error', error, { once: true });
      pendingImages.push({ img, load, error });
    }
  };

  let currentMediaCount = 0;
  refreshBindings();

  let mutationObserver: MutationObserver | null = null;

  // Mermaid 可能在首次绑定后才异步完成渲染。
  if (typeof windowRef?.MutationObserver === 'function') {
    mutationObserver = new windowRef.MutationObserver(() => refreshBindings());
    mutationObserver.observe(article, { childList: true, subtree: true });
  }

  overlay.addEventListener('pointerdown', onOverlayPointerDown);
  wrap.addEventListener('wheel', onWheel, { passive: false });
  wrap.addEventListener('pointerdown', onPointerDown);
  wrap.addEventListener('pointermove', onPointerMove);
  wrap.addEventListener('pointerup', onPointerUp);
  wrap.addEventListener('pointercancel', onPointerUp);
  wrap.addEventListener('dblclick', onDoubleClick);
  closeBtn.addEventListener('click', () => close());
  zoomInBtn.addEventListener('click', () => setScaleCentered(state.scale * ZOOM_STEP, 'manual'));
  zoomOutBtn.addEventListener('click', () => setScaleCentered(state.scale / ZOOM_STEP, 'manual'));
  resetBtn.addEventListener('click', () => setScaleCentered(1, 'manual'));
  fitBtn.addEventListener('click', () => fitToViewport());

  const destroy = () => {
    if (destroyed) return;
    destroyed = true;
    close();
    clearBindings();
    mutationObserver?.disconnect();
    mutationObserver = null;
    overlay.removeEventListener('pointerdown', onOverlayPointerDown);
    wrap.removeEventListener('wheel', onWheel);
    wrap.removeEventListener('pointerdown', onPointerDown);
    wrap.removeEventListener('pointermove', onPointerMove);
    wrap.removeEventListener('pointerup', onPointerUp);
    wrap.removeEventListener('pointercancel', onPointerUp);
    wrap.removeEventListener('dblclick', onDoubleClick);
    overlay.remove();
  };

  return {
    destroy,
    isOpen: () => active,
    mediaCount: () => currentMediaCount,
  };
};

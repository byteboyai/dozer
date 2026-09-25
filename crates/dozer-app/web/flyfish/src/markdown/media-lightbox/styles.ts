// Markdown 媒体灯箱样式。以独立 <style> 注入,只在 Flyfish 渲染 surface 内
// 竞争 z-index,不使用 iced 遮挡处理。
//
// 主题:深色对齐 ByteBoy2077 `#0a0e16`,但控件普通状态**不用**甲方动作专属
// 金色。用 [data-viewer-theme='dark'](renderer 既有的主题标记)切换。

export const LIGHTBOX_STYLE_ID = 'dozer-markdown-media-lightbox-style';

export const markdownMediaLightboxStyle = `
.markdown-body img:not([data-dozer-lightbox-skip]),
.markdown-body .markdown-mermaid[data-markdown-mermaid='rendered'] > svg{cursor:zoom-in}
.markdown-body img:not([data-dozer-lightbox-skip]):focus-visible,
.markdown-body .markdown-mermaid[data-markdown-mermaid='rendered'] > svg:focus-visible{outline:2px solid #47def0;outline-offset:2px;border-radius:4px}

.dozer-media-lightbox{position:fixed;inset:0;z-index:2147483000;display:flex;align-items:center;justify-content:center;background:rgba(10,14,22,.92);color:#ffe5b4;-webkit-user-select:none;user-select:none;touch-action:none;overscroll-behavior:contain}
.dozer-media-lightbox[hidden]{display:none}
.dozer-media-lightbox__stage-wrap{position:absolute;inset:0;overflow:hidden}
.dozer-media-lightbox__stage{position:absolute;left:0;top:0;transform-origin:0 0;will-change:transform;cursor:grab}
.dozer-media-lightbox__stage.is-dragging{cursor:grabbing}
.dozer-media-lightbox__stage.is-zoomed{cursor:grab}
.dozer-media-lightbox__media{display:block;width:100%;height:100%;max-width:none;max-height:none;pointer-events:none;-webkit-user-drag:none;user-select:none}
.dozer-media-lightbox__media--image{background:#0a0e16}
.dozer-media-lightbox__toolbar{position:absolute;top:16px;left:50%;transform:translateX(-50%);display:flex;align-items:center;gap:6px;padding:6px;border-radius:999px;background:rgba(10,14,22,.82);border:1px solid rgba(255,229,180,.22);box-shadow:0 8px 28px rgba(0,0,0,.45);z-index:1}
.dozer-media-lightbox__btn{appearance:none;border:0;background:transparent;color:#ffe5b4;width:34px;height:34px;border-radius:999px;display:inline-flex;align-items:center;justify-content:center;cursor:pointer;font-size:16px;line-height:1;padding:0}
.dozer-media-lightbox__btn:hover{background:rgba(255,229,180,.14)}
.dozer-media-lightbox__btn:focus-visible{outline:2px solid #47def0;outline-offset:2px}
.dozer-media-lightbox__btn[disabled]{opacity:.4;cursor:default;background:transparent}
.dozer-media-lightbox__btn--close{margin-left:4px}
.dozer-media-lightbox__zoom{min-width:56px;text-align:center;font-variant-numeric:tabular-nums;font-size:13px;color:#ffe5b4;padding:0 4px}
.dozer-media-lightbox__sr{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap;border:0}

[data-viewer-theme='light'] .dozer-media-lightbox,
[data-viewer-theme='system'] .dozer-media-lightbox{background:rgba(10,14,22,.86)}
`;

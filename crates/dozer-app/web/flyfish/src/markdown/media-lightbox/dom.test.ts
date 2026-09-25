import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, test } from 'node:test';
import { JSDOM, type DOMWindow } from 'jsdom';

import { collectMedia, installMarkdownMediaLightbox } from './index.ts';
import { rewriteSvgIds } from './svgIds.ts';

interface Harness {
  window: DOMWindow;
  document: Document;
  viewer: HTMLElement;
  article: HTMLElement;
}

let harness: Harness;
let handles: Array<{ destroy: () => void }> = [];

const buildHarness = (body = ''): Harness => {
  const dom = new JSDOM(
    `<!doctype html><html><head></head><body><div class="markdown-viewer"><article class="markdown-body">${body}</article></div></body></html>`,
    { pretendToBeVisual: true }
  );
  const window = dom.window;
  const document = window.document;
  const viewer = document.querySelector('.markdown-viewer') as HTMLElement;
  const article = document.querySelector('.markdown-body') as HTMLElement;
  return { window, document, viewer, article };
};

const fakeImage = (
  document: Document,
  attrs: Record<string, string>,
  natural: { width: number; height: number } | null
): HTMLImageElement => {
  const img = document.createElement('img');
  for (const [k, v] of Object.entries(attrs)) img.setAttribute(k, v);
  if (natural) {
    Object.defineProperty(img, 'complete', { value: true, configurable: true });
    Object.defineProperty(img, 'naturalWidth', { value: natural.width, configurable: true });
    Object.defineProperty(img, 'naturalHeight', { value: natural.height, configurable: true });
  } else {
    Object.defineProperty(img, 'complete', { value: false, configurable: true });
    Object.defineProperty(img, 'naturalWidth', { value: 0, configurable: true });
    Object.defineProperty(img, 'naturalHeight', { value: 0, configurable: true });
  }
  return img;
};

const install = () => {
  const handle = installMarkdownMediaLightbox({
    viewer: harness.viewer,
    article: harness.article,
  });
  handles.push(handle);
  return handle;
};

const click = (el: Element) => {
  el.dispatchEvent(new harness.window.MouseEvent('click', { bubbles: true, cancelable: true }));
};

const key = (target: EventTarget, k: string) => {
  target.dispatchEvent(
    new harness.window.KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
  );
};

beforeEach(() => {
  handles = [];
});

afterEach(() => {
  for (const h of handles) h.destroy();
  handles = [];
});

describe('collectMedia', () => {
  test('only includes loaded images', () => {
    harness = buildHarness();
    const loaded = fakeImage(harness.document, { alt: 'Loaded' }, { width: 100, height: 50 });
    const pending = fakeImage(harness.document, { alt: 'Pending' }, null);
    harness.article.append(loaded, pending);
    const media = collectMedia(harness.article, harness.document);
    assert.equal(media.length, 1);
    assert.equal(media[0].kind, 'image');
    assert.equal(media[0].source, loaded);
    assert.equal(media[0].accessibleName, 'Loaded');
    assert.equal(media[0].intrinsicWidth, 100);
  });

  test('uses fallback accessible name when alt missing', () => {
    harness = buildHarness();
    harness.article.append(fakeImage(harness.document, {}, { width: 10, height: 10 }));
    const media = collectMedia(harness.article, harness.document);
    assert.equal(media[0].accessibleName, 'Image');
  });

  test('includes only rendered mermaid svg, not error source blocks', () => {
    harness = buildHarness(`
      <div class="markdown-mermaid" data-markdown-mermaid="rendered"><svg aria-label="Diagram A"></svg></div>
      <pre class="markdown-mermaid-source-error"><code class="language-mermaid">graph</code></pre>
    `);
    const media = collectMedia(harness.article, harness.document);
    assert.equal(media.length, 1);
    assert.equal(media[0].kind, 'mermaid');
    assert.equal(media[0].accessibleName, 'Diagram A');
  });

  test('skips marked skip images', () => {
    harness = buildHarness();
    const img = fakeImage(harness.document, { 'data-dozer-lightbox-skip': '' }, { width: 5, height: 5 });
    harness.article.append(img);
    assert.equal(collectMedia(harness.article, harness.document).length, 0);
  });
});

describe('open/close', () => {
  test('open by clicking an image, close with Escape restores focus and scroll', () => {
    harness = buildHarness();
    harness.viewer.style.overflow = 'auto';
    harness.viewer.scrollTop = 42;
    const img = fakeImage(harness.document, { alt: 'Pic' }, { width: 800, height: 600 });
    harness.article.append(img);
    const handle = install();

    click(img);
    assert.equal(handle.isOpen(), true);
    const overlay = harness.document.querySelector('.dozer-media-lightbox') as HTMLElement;
    assert.ok(overlay && overlay.hidden === false);
    assert.equal(harness.viewer.style.overflow, 'hidden');
    assert.equal(overlay.querySelectorAll('.dozer-media-lightbox__media').length, 1);

    key(harness.document, 'Escape');
    assert.equal(handle.isOpen(), false);
    assert.equal(harness.viewer.style.overflow, 'auto');
    assert.equal(harness.viewer.scrollTop, 42);
    assert.ok(!harness.document.querySelector('.dozer-media-lightbox'));
  });

  test('Enter on focused image opens', () => {
    harness = buildHarness();
    const img = fakeImage(harness.document, {}, { width: 10, height: 10 });
    harness.article.append(img);
    const handle = install();
    key(img, 'Enter');
    assert.equal(handle.isOpen(), true);
  });

  test('close button closes; background click only when target is overlay', () => {
    harness = buildHarness();
    harness.article.append(fakeImage(harness.document, {}, { width: 10, height: 10 }));
    const handle = install();
    click(harness.article.querySelector('img')!);
    const overlay = harness.document.querySelector('.dozer-media-lightbox') as HTMLElement;

    // 媒体本体上的 pointerdown 不关闭。
    overlay
      .querySelector('.dozer-media-lightbox__stage')!
      .dispatchEvent(new harness.window.Event('pointerdown', { bubbles: true }));
    assert.equal(handle.isOpen(), true);

    // 背景(target === overlay)关闭。
    overlay.dispatchEvent(new harness.window.Event('pointerdown', { bubbles: true }));
    assert.equal(handle.isOpen(), false);

    // 再开一次,用关闭按钮关。
    click(harness.article.querySelector('img')!);
    click(harness.document.querySelector('.dozer-media-lightbox__btn--close')!);
    assert.equal(handle.isOpen(), false);
  });

  test('mermaid opens a cloned svg and source is untouched', () => {
    harness = buildHarness(
      `<div class="markdown-mermaid" data-markdown-mermaid="rendered"><svg aria-label="D"><defs><marker id="m"></marker></defs><path marker-end="url(#m)"/></svg></div>`
    );
    const handle = install();
    click(harness.article.querySelector('svg')!);
    const clone = harness.document.querySelector('.dozer-media-lightbox__media') as SVGSVGElement;
    assert.ok(clone);
    assert.notEqual(clone, harness.article.querySelector('svg'));
    // 正文原始 id 未被改动。
    assert.equal(harness.article.querySelector('marker')!.getAttribute('id'), 'm');
  });
});

describe('keyboard/wheel controls', () => {
  const openSmall = () => {
    harness = buildHarness();
    harness.article.append(fakeImage(harness.document, {}, { width: 2000, height: 2000 }));
    const handle = install();
    click(harness.article.querySelector('img')!);
    const overlay = harness.document.querySelector('.dozer-media-lightbox') as HTMLElement;
    const label = () => overlay.querySelector('.dozer-media-lightbox__zoom')!.textContent;
    return { handle, overlay, label };
  };

  test('+/-/0/f update zoom and never exceed 10%-500%', () => {
    const { overlay, label } = openSmall();
    key(harness.document, '+');
    assert.notEqual(label(), '100%');
    key(harness.document, '0');
    assert.equal(label(), '100%');
    key(harness.document, 'f');
    // 视口为 0 时 fit 回退 100%。
    assert.equal(label(), '100%');
    for (let i = 0; i < 40; i += 1) key(harness.document, '+');
    const pct = parseInt(label() ?? '0', 10);
    assert.ok(pct <= 500, `pct ${pct} must be <= 500`);
    for (let i = 0; i < 80; i += 1) key(harness.document, '-');
    const pctLow = parseInt(label() ?? '0', 10);
    assert.ok(pctLow >= 10, `pct ${pctLow} must be >= 10`);
    void overlay;
  });

  test('plain wheel does not zoom, meta/ctrl wheel does', () => {
    const { overlay, label } = openSmall();
    const before = label();
    overlay
      .querySelector('.dozer-media-lightbox__stage-wrap')!
      .dispatchEvent(
        new harness.window.WheelEvent('wheel', { deltaY: -100, bubbles: true, cancelable: true })
      );
    assert.equal(label(), before);
    overlay
      .querySelector('.dozer-media-lightbox__stage-wrap')!
      .dispatchEvent(
        new harness.window.WheelEvent('wheel', {
          deltaY: -100,
          metaKey: true,
          bubbles: true,
          cancelable: true,
        })
      );
    assert.notEqual(label(), before);
  });

  test('editable target does not receive zoom shortcut handling', () => {
    const { overlay, label } = openSmall();
    const input = harness.document.createElement('input');
    overlay.appendChild(input);
    const before = label();
    key(input, '+');
    assert.equal(label(), before);
  });
});

describe('lifecycle', () => {
  test('destroy is idempotent and removes overlay/listeners', () => {
    harness = buildHarness();
    harness.article.append(fakeImage(harness.document, {}, { width: 10, height: 10 }));
    const handle = install();
    click(harness.article.querySelector('img')!);
    handle.destroy();
    handle.destroy();
    assert.equal(handle.isOpen(), false);
    assert.ok(!harness.document.querySelector('.dozer-media-lightbox'));
    // 解绑后点击不再开。
    click(harness.article.querySelector('img')!);
    assert.equal(handle.isOpen(), false);
  });

  test('repeated open/close does not accumulate overlay nodes', () => {
    harness = buildHarness();
    harness.article.append(fakeImage(harness.document, {}, { width: 10, height: 10 }));
    const handle = install();
    const img = harness.article.querySelector('img')!;
    for (let i = 0; i < 50; i += 1) {
      click(img);
      key(harness.document, 'Escape');
    }
    assert.equal(harness.document.querySelectorAll('.dozer-media-lightbox').length, 0);
    assert.equal(handle.isOpen(), false);
  });
});

describe('rewriteSvgIds', () => {
  test('prefixes ids and references without prefix collisions', () => {
    const dom = new JSDOM(
      `<svg xmlns="http://www.w3.org/2000/svg"><defs><marker id="a"></marker><marker id="ab"></marker></defs><path marker-end="url(#ab)" fill="url(#a)"/><use href="#a"/></svg>`
    );
    const svg = dom.window.document.querySelector('svg')!;
    const { rewritten } = rewriteSvgIds(svg, 'p-');
    assert.equal(rewritten, 2);
    assert.equal(svg.querySelector('[id="p-a"]')!.id, 'p-a');
    assert.equal(svg.querySelector('[id="p-ab"]')!.id, 'p-ab');
    assert.equal(svg.querySelector('path')!.getAttribute('marker-end'), 'url(#p-ab)');
    assert.equal(svg.querySelector('path')!.getAttribute('fill'), 'url(#p-a)');
    assert.equal(svg.querySelector('use')!.getAttribute('href'), '#p-a');
  });

  test('no ids means no rewrite', () => {
    const dom = new JSDOM(`<svg xmlns="http://www.w3.org/2000/svg"><path/></svg>`);
    const { rewritten } = rewriteSvgIds(dom.window.document.querySelector('svg')!, 'p-');
    assert.equal(rewritten, 0);
  });
});

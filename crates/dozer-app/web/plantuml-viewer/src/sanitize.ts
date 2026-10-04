//! Sanitizer for engine-produced SVG before it is mounted (spec §6.4).
//!
//! The engine output is treated as untrusted: it can embed `<script>`,
//! `on*` event attributes, `<foreignObject>`, external URLs and `javascript:`
//! references. The first version strips scripts / event handlers /
//! foreignObject and disables all in-diagram links.

const SVG_NS = "http://www.w3.org/2000/svg";

/** Spec §6.3: reject SVG output larger than 32 MiB before parsing. */
export const MAX_SVG_BYTES = 32 * 1024 * 1024;

/** Elements that must never appear in mounted output. */
const BANNED_ELEMENTS = new Set(["script", "foreignobject", "iframe", "object", "embed"]);

/** URL-bearing attributes that must not point off-document. */
const URL_ATTRS = new Set(["href", "xlink:href", "src", "xlink:src"]);

export interface SanitizeResult {
  svg: SVGSVGElement;
  /** Number of nodes/attributes removed; useful for diagnostics/tests. */
  removed: number;
}

function isSafeUrl(value: string): boolean {
  const v = value.trim().toLowerCase();
  // Only same-document fragment refs are allowed (links disabled anyway).
  return v.startsWith("#") || v === "";
}

function walk(el: Element, onRemove: () => void): void {
  // Depth-first, remove banned subtrees.
  const children = Array.from(el.children);
  for (const child of children) {
    if (BANNED_ELEMENTS.has(child.tagName.toLowerCase())) {
      child.remove();
      onRemove();
      continue;
    }
    walk(child, onRemove);
  }
  // Strip event handlers and unsafe URL attributes on the element itself.
  for (const attr of Array.from(el.attributes)) {
    const name = attr.name.toLowerCase();
    if (name.startsWith("on")) {
      el.removeAttribute(attr.name);
      onRemove();
      continue;
    }
    if (URL_ATTRS.has(name) && !isSafeUrl(attr.value)) {
      el.removeAttribute(attr.name);
      onRemove();
      continue;
    }
  }
  // First version disables all in-diagram links: drop <a> hrefs entirely.
  if (el.tagName.toLowerCase() === "a") {
    el.removeAttribute("href");
    el.removeAttribute("xlink:href");
  }
}

/** Parse and sanitize an SVG string into a live SVG element. */
export function sanitizeSvg(svgText: string): SanitizeResult {
  // Byte length of the UTF-8 encoding, matching the Rust-side cap.
  if (new TextEncoder().encode(svgText).length > MAX_SVG_BYTES) {
    throw new Error(`SVG 输出超过上限 ${MAX_SVG_BYTES} 字节`);
  }
  const doc = new DOMParser().parseFromString(svgText, "image/svg+xml");
  const parseError = doc.querySelector("parsererror");
  if (parseError || doc.documentElement.tagName.toLowerCase() !== "svg") {
    throw new Error("引擎输出不是合法 SVG");
  }
  const svg = doc.documentElement as unknown as SVGSVGElement;
  let removed = 0;
  walk(svg, () => {
    removed += 1;
  });
  // Re-serialize so the returned node belongs to the live document.
  const clean = new DOMParser().parseFromString(
    new XMLSerializer().serializeToString(svg),
    "image/svg+xml",
  ).documentElement as unknown as SVGSVGElement;
  clean.setAttribute("xmlns", SVG_NS);
  clean.removeAttribute("data-plantuml-src");
  return { svg: clean, removed };
}

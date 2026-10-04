//! SVG viewport interaction: wheel zoom, drag pan, Fit, Actual Size, Reset.
//! Pure view state — never persisted (spec §7).

const MIN_SCALE = 0.1;
const MAX_SCALE = 40;
const ZOOM_STEP = 1.15;

export interface ViewportState {
  scale: number;
  x: number;
  y: number;
}

export class Viewport {
  private readonly host: HTMLElement;
  private readonly content: HTMLElement;
  private state: ViewportState = { scale: 1, x: 0, y: 0 };
  private dragging = false;
  private dragStart = { x: 0, y: 0, tx: 0, ty: 0 };
  private contentSize = { width: 0, height: 0 };

  constructor(host: HTMLElement, content: HTMLElement) {
    this.host = host;
    this.content = content;
    this.host.addEventListener("wheel", this.onWheel, { passive: false });
    this.host.addEventListener("mousedown", this.onMouseDown);
    window.addEventListener("mousemove", this.onMouseMove);
    window.addEventListener("mouseup", this.onMouseUp);
  }

  setContentSize(width: number, height: number): void {
    this.contentSize = { width, height };
  }

  getState(): ViewportState {
    return { ...this.state };
  }

  private apply(): void {
    const { scale, x, y } = this.state;
    this.content.style.transform = `translate(${x}px, ${y}px) scale(${scale})`;
  }

  private onWheel = (ev: WheelEvent): void => {
    ev.preventDefault();
    const rect = this.host.getBoundingClientRect();
    const cx = ev.clientX - rect.left;
    const cy = ev.clientY - rect.top;
    const factor = ev.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
    this.zoomAt(cx, cy, factor);
  };

  private zoomAt(cx: number, cy: number, factor: number): void {
    const next = clamp(this.state.scale * factor, MIN_SCALE, MAX_SCALE);
    const ratio = next / this.state.scale;
    // Keep the point under the cursor fixed.
    this.state.x = cx - (cx - this.state.x) * ratio;
    this.state.y = cy - (cy - this.state.y) * ratio;
    this.state.scale = next;
    this.apply();
  }

  zoomIn(): void {
    const rect = this.host.getBoundingClientRect();
    this.zoomAt(rect.width / 2, rect.height / 2, ZOOM_STEP);
  }

  zoomOut(): void {
    const rect = this.host.getBoundingClientRect();
    this.zoomAt(rect.width / 2, rect.height / 2, 1 / ZOOM_STEP);
  }

  actualSize(): void {
    this.state = { scale: 1, x: 0, y: 0 };
    this.apply();
  }

  reset(): void {
    this.fit();
  }

  fit(): void {
    const rect = this.host.getBoundingClientRect();
    const w = this.contentSize.width || 1;
    const h = this.contentSize.height || 1;
    if (rect.width === 0 || rect.height === 0) {
      return;
    }
    const scale = Math.min(rect.width / w, rect.height / h) * 0.98;
    const clamped = clamp(scale, MIN_SCALE, MAX_SCALE);
    this.state = {
      scale: clamped,
      x: (rect.width - w * clamped) / 2,
      y: (rect.height - h * clamped) / 2,
    };
    this.apply();
  }

  private onMouseDown = (ev: MouseEvent): void => {
    if (ev.button !== 0) {
      return;
    }
    this.dragging = true;
    this.dragStart = { x: ev.clientX, y: ev.clientY, tx: this.state.x, ty: this.state.y };
    this.host.style.cursor = "grabbing";
    ev.preventDefault();
  };

  private onMouseMove = (ev: MouseEvent): void => {
    if (!this.dragging) {
      return;
    }
    this.state.x = this.dragStart.tx + (ev.clientX - this.dragStart.x);
    this.state.y = this.dragStart.ty + (ev.clientY - this.dragStart.y);
    this.apply();
  };

  private onMouseUp = (): void => {
    if (!this.dragging) {
      return;
    }
    this.dragging = false;
    this.host.style.cursor = "";
  };

  dispose(): void {
    this.host.removeEventListener("wheel", this.onWheel);
    this.host.removeEventListener("mousedown", this.onMouseDown);
    window.removeEventListener("mousemove", this.onMouseMove);
    window.removeEventListener("mouseup", this.onMouseUp);
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

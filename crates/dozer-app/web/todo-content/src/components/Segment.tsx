import type { ComponentChildren } from 'preact';

export function SegmentDivider({ label }: { label: string }) {
  return (
    <div class="segment-divider">
      <span class="segment-divider-label">{label}</span>
      <span class="segment-divider-line" />
    </div>
  );
}

export function EmptyHint({ children }: { children: ComponentChildren }) {
  return <div class="empty-hint">{children}</div>;
}

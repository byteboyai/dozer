export function AvatarIcon({ svg, color }: { svg: string; color?: string | null }) {
  return (
    <span
      class="avatar-icon"
      style={color ? { color } : undefined}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}

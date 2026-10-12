interface Stat {
  label: string;
  value: string;
  colorVar: string;
}

export function StatBox({ stats }: { stats: Stat[] }) {
  return (
    <div class="usage-stat-box">
      {stats.map((s) => (
        <div class="usage-stat" key={s.label}>
          <span class="usage-stat-label">{s.label}</span>
          <span class="usage-stat-value" style={{ color: `var(${s.colorVar})` }}>
            {s.value}
          </span>
        </div>
      ))}
    </div>
  );
}

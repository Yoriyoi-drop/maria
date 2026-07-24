export default function CoverageTab() {
  const items = [
    { label: "Statement", value: 98, color: "var(--accent-green)" },
    { label: "Branch", value: 96, color: "var(--accent-green)" },
    { label: "Toggle", value: 95, color: "var(--accent-cyan)" },
    { label: "FSM", value: 99, color: "var(--accent-green)" },
    { label: "Assertion", value: 100, color: "var(--accent-green)" },
    { label: "Function", value: 92, color: "var(--accent-yellow)" },
  ];

  return (
    <div>
      <div className="metrics-grid">
        {items.map((item) => (
          <div key={item.label} className="metric-card">
            <div className="metric-card__label">{item.label}</div>
            <div className="metric-card__value" style={{ color: item.color }}>
              {item.value}%
            </div>
            <div style={{ marginTop: 6, height: 4, background: "var(--bg-tertiary)", borderRadius: 2 }}>
              <div style={{ width: `${item.value}%`, height: "100%", background: item.color, borderRadius: 2 }} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
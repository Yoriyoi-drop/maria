export default function LspTab() {
  const items = [
    { status: "connected", label: "Maria Language Server", detail: "v0.1.0 — 47 modules indexed" },
    { status: "idle", label: "Semantic Tokens", detail: "312 signals, 54 types, 23 modules" },
    { status: "idle", label: "Diagnostics", detail: "0 errors, 2 warnings, 3 hints" },
    { status: "idle", label: "Code Actions", detail: "Available: organize imports, generate module" },
  ];

  return (
    <div>
      {items.map((item, i) => (
        <div key={i} className="diag-item">
          <span
            style={{
              width: 8,
              height: 8,
              borderRadius: "50%",
              background: item.status === "connected" ? "var(--accent-green)" : "var(--text-muted)",
              flexShrink: 0,
              marginTop: 4,
            }}
          />
          <div>
            <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>{item.label}</div>
            <div style={{ color: "var(--text-tertiary)", fontSize: 11 }}>{item.detail}</div>
          </div>
        </div>
      ))}
    </div>
  );
}
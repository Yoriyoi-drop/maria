import { useState } from "react";
import { ArrowDown, Package } from "lucide-react";
import useProjectStore from "../../stores/projectStore";

export default function DependencyView() {
  const { modules } = useProjectStore();

  if (modules.length === 0) {
    return (
      <div className="sidebar-section">
        <p style={{ padding: "12px", color: "var(--text-tertiary)", fontSize: 12 }}>
          Compile to see dependency graph
        </p>
      </div>
    );
  }

  return (
    <div>
      <div className="sidebar-section">
        <div className="sidebar-section__title">Module Dependencies</div>
      </div>
      {modules.slice(0, 20).map((mod, i) => (
        <div key={i} className="sidebar-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 2 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <Package size={13} className="sidebar-item__icon" />
            <span className="sidebar-item__name">{mod.name}</span>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 4, paddingLeft: 19 }}>
            <ArrowDown size={10} style={{ color: "var(--text-muted)" }} />
            <span style={{ fontSize: 11, color: "var(--text-tertiary)" }}>{mod.file}</span>
          </div>
        </div>
      ))}
    </div>
  );
}
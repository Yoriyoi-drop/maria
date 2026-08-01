import { useMemo } from "react";
import { Package, Variable, FunctionSquare, Type, Shield, Puzzle } from "lucide-react";
import useProjectStore from "../../stores/projectStore";

const iconMap: Record<string, any> = {
  module: Puzzle,
  interface: Shield,
  package: Package,
  class: Type,
  function: FunctionSquare,
  task: Variable,
};

export default function SymbolsView() {
  const { modules } = useProjectStore();

  const grouped = useMemo(() => {
    const map: Record<string, typeof modules> = {};
    for (const m of modules) {
      if (!map[m.kind]) map[m.kind] = [];
      map[m.kind].push(m);
    }
    return map;
  }, [modules]);

  if (modules.length === 0) {
    return (
      <div className="sidebar-section">
        <p style={{ padding: "12px", color: "var(--text-tertiary)", fontSize: 12 }}>
          Compile to see symbols
        </p>
      </div>
    );
  }

  return (
    <div>
      {Object.entries(grouped).map(([kind, items]) => (
        <div key={kind} className="sidebar-section">
          <div className="sidebar-section__title">
            {kind.charAt(0).toUpperCase() + kind.slice(1)}s
            <span className="sidebar-item__badge" style={{ marginLeft: 6 }}>{items.length}</span>
          </div>
          {items.map((item, i) => {
            const Icon = iconMap[kind] || Puzzle;
            return (
              <div key={i} className="sidebar-item">
                <Icon size={13} className="sidebar-item__icon" />
                <span className="sidebar-item__name">{item.name}</span>
                <span style={{ fontSize: 10, color: "var(--text-muted)" }}>{item.file}</span>
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}
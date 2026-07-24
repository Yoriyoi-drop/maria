import { useState } from "react";
import { Search, File, X } from "lucide-react";

export default function SearchView() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<{ file: string; line: number; text: string }[]>([]);

  return (
    <div>
      <div className="sidebar-section">
        <div className="sidebar-section__title">Search</div>
        <div style={{ position: "relative" }}>
          <Search size={13} style={{ position: "absolute", left: 8, top: 7, color: "var(--text-muted)" }} />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Find module, signal, parameter..."
            style={{
              width: "100%",
              padding: "5px 8px 5px 28px",
              borderRadius: 4,
              fontSize: 12,
            }}
          />
          {query && (
            <button
              onClick={() => setQuery("")}
              style={{ position: "absolute", right: 6, top: 6, color: "var(--text-muted)" }}
            >
              <X size={13} />
            </button>
          )}
        </div>
      </div>

      <div className="sidebar-section">
        <div className="sidebar-section__title">Filters</div>
        {["module", "signal", "parameter", "package", "macro", "instance"].map((f) => (
          <label key={f} className="sidebar-item" style={{ gap: 6, cursor: "pointer" }}>
            <input type="checkbox" defaultChecked style={{ accentColor: "var(--accent-blue)" }} />
            <span>{f.charAt(0).toUpperCase() + f.slice(1)}</span>
          </label>
        ))}
      </div>

      {results.length > 0 && (
        <div className="sidebar-section">
          <div className="sidebar-section__title">Results ({results.length})</div>
          {results.map((r, i) => (
            <div key={i} className="sidebar-item">
              <File size={12} className="sidebar-item__icon" />
              <span className="sidebar-item__name">{r.file}:{r.line}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
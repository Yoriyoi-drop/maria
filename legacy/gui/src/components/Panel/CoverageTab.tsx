import { useState } from "react";
import { File, ExternalLink } from "lucide-react";
import useEditorStore from "../../stores/editorStore";

interface CoverageItem {
  label: string;
  value: number;
  color: string;
  details?: { file: string; line: number; name: string; covered: boolean }[];
}

const mockDetails: Record<string, { file: string; line: number; name: string; covered: boolean }[]> = {
  Statement: [
    { file: "core/alu.sv", line: 47, name: "alu.sv:47 — assign result = a + b", covered: true },
    { file: "core/alu.sv", line: 52, name: "alu.sv:52 — assign result = a - b", covered: true },
    { file: "core/alu.sv", line: 58, name: "alu.sv:58 — assign result = a & b", covered: true },
    { file: "core/alu.sv", line: 63, name: "alu.sv:63 — assign result = a | b", covered: true },
    { file: "core/alu.sv", line: 68, name: "alu.sv:68 — default: result = '0", covered: false },
  ],
  Branch: [
    { file: "core/cache_ctrl.sv", line: 112, name: "cache_ctrl.sv:112 — if (hit)", covered: true },
    { file: "core/cache_ctrl.sv", line: 118, name: "cache_ctrl.sv:118 — else // miss", covered: true },
    { file: "core/cache_ctrl.sv", line: 134, name: "cache_ctrl.sv:134 — if (dirty)", covered: true },
    { file: "core/cache_ctrl.sv", line: 140, name: "cache_ctrl.sv:140 — else // clean evict", covered: false },
  ],
  FSM: [
    { file: "core/cache_ctrl.sv", line: 28, name: "IDLE → LOOKUP", covered: true },
    { file: "core/cache_ctrl.sv", line: 29, name: "LOOKUP → HIT", covered: true },
    { file: "core/cache_ctrl.sv", line: 30, name: "LOOKUP → MISS", covered: true },
    { file: "core/cache_ctrl.sv", line: 31, name: "MISS → FILL", covered: true },
    { file: "core/cache_ctrl.sv", line: 32, name: "FILL → IDLE", covered: true },
  ],
};

export default function CoverageTab() {
  const [expanded, setExpanded] = useState<string | null>(null);
  const { openFile } = useEditorStore();

  const items: CoverageItem[] = [
    { label: "Statement", value: 98, color: "var(--accent-green)", details: mockDetails.Statement },
    { label: "Branch", value: 96, color: "var(--accent-yellow)", details: mockDetails.Branch },
    { label: "Toggle", value: 95, color: "var(--accent-cyan)" },
    { label: "FSM", value: 99, color: "var(--accent-green)", details: mockDetails.FSM },
    { label: "Assertion", value: 100, color: "var(--accent-green)" },
    { label: "Function", value: 92, color: "var(--accent-yellow)" },
  ];

  const handleNavigate = (file: string, line: number) => {
    const fileName = file.split("/").pop() || file;
    openFile(file, fileName);
  };

  return (
    <div className="fade-in">
      {/* Summary header */}
      <div className="panel-section">
        <div className="panel-section__title">Coverage Overview</div>
      </div>

      {/* Metric cards */}
      <div className="metrics-grid">
        {items.map((item) => (
          <div
            key={item.label}
            className="metric-card"
            style={{ cursor: item.details ? "pointer" : "default" }}
            onClick={() => setExpanded(expanded === item.label ? null : item.label)}
          >
            <div className="metric-card__label">{item.label}</div>
            <div className="metric-card__value" style={{ color: item.color }}>
              {item.value}%
            </div>
            <div className="cov-bar">
              <div className="cov-bar__track">
                <div
                  className="cov-bar__fill"
                  style={{ width: `${item.value}%`, background: item.color }}
                />
              </div>
            </div>
            {item.details && (
              <div className="cov-expand-hint">
                {expanded === item.label ? "Click to collapse" : "Click for details"}
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Expanded details with source navigation */}
      {expanded && mockDetails[expanded] && (
        <div className="cov-details fade-in">
          <div className="panel-section">
            <div className="panel-section__title">
              {expanded} Coverage — Detailed View
              <span style={{ fontWeight: 400, fontSize: 10, marginLeft: 8 }}>
                (click any item to open source)
              </span>
            </div>
          </div>
          {mockDetails[expanded].map((d, i) => (
            <div
              key={i}
              className="cov-detail-item"
              title={`Open ${d.file}:${d.line} in editor`}
              style={{ cursor: "pointer" }}
              onClick={() => handleNavigate(d.file, d.line)}
            >
              <span
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: "50%",
                  background: d.covered ? "var(--accent-green)" : "var(--accent-red)",
                  flexShrink: 0,
                }}
              />
              <File size={12} style={{ color: "var(--text-muted)", flexShrink: 0 }} />
              <span style={{ color: "var(--text-tertiary)", fontSize: 11, flexShrink: 0 }}>
                {d.file}:{d.line}
              </span>
              <span
                style={{
                  color: d.covered ? "var(--text-secondary)" : "var(--accent-red)",
                  fontSize: 12,
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                }}
              >
                {d.name}
              </span>
              <span
                style={{
                  fontSize: 10,
                  color: d.covered ? "var(--accent-green)" : "var(--accent-red)",
                  flexShrink: 0,
                  marginLeft: "auto",
                  display: "flex",
                  alignItems: "center",
                  gap: 4,
                }}
              >
                {d.covered ? "covered" : "not covered"}
                <ExternalLink size={10} style={{ opacity: 0.5 }} />
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

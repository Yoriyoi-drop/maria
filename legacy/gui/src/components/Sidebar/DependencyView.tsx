import { useState } from "react";
import {
  ArrowDownRight, Cpu, Database, GitMerge, Monitor,
  Zap, Network, Layers, HardDrive, Box
} from "lucide-react";
import useEditorStore from "../../stores/editorStore";

interface DepEdge {
  from: string;
  to: string;
  kind: "instantiates" | "imports" | "uses";
}

interface DepNode {
  name: string;
  kind: string;
  file: string;
  children: DepNode[];
  icon: any;
  color: string;
}

function kindIcon(kind: string) {
  const k = kind.toLowerCase();
  if (k.includes("cpu") || k.includes("core") || k.includes("decoder") || k.includes("alu")) return Cpu;
  if (k.includes("cache") || k.includes("memory") || k.includes("sram") || k.includes("ddr")) return Database;
  if (k.includes("interconnect") || k.includes("bus") || k.includes("axi") || k.includes("crossbar")) return GitMerge;
  if (k.includes("controller") || k.includes("ctrl")) return Zap;
  if (k.includes("io") || k.includes("gpio") || k.includes("uart") || k.includes("spi") || k.includes("i2c")) return Monitor;
  if (k.includes("clock") || k.includes("clk")) return HardDrive;
  if (k.includes("fifo") || k.includes("buffer")) return Layers;
  if (k.includes("network") || k.includes("noc") || k.includes("router")) return Network;
  return Box;
}

function kindColor(kind: string): string {
  const k = kind.toLowerCase();
  if (k.includes("cpu") || k.includes("core")) return "var(--accent-blue)";
  if (k.includes("cache") || k.includes("memory") || k.includes("ddr")) return "var(--accent-green)";
  if (k.includes("interconnect") || k.includes("bus") || k.includes("axi")) return "var(--accent-purple)";
  if (k.includes("controller") || k.includes("ctrl")) return "var(--accent-cyan)";
  if (k.includes("io") || k.includes("uart") || k.includes("gpio")) return "var(--accent-orange)";
  if (k.includes("clock") || k.includes("clk")) return "var(--accent-yellow)";
  if (k.includes("verify") || k.includes("tb") || k.includes("test")) return "var(--accent-teal)";
  return "var(--text-secondary)";
}

// ── Mock dependency data ──
const mockDeps: DepEdge[] = [
  { from: "Aurora-172 SoC", to: "CPU Core", kind: "instantiates" },
  { from: "Aurora-172 SoC", to: "Cache System", kind: "instantiates" },
  { from: "Aurora-172 SoC", to: "Interconnect", kind: "instantiates" },
  { from: "Aurora-172 SoC", to: "Memory Controller", kind: "instantiates" },
  { from: "Aurora-172 SoC", to: "Peripherals", kind: "instantiates" },
  { from: "CPU Core", to: "Instruction Decoder", kind: "instantiates" },
  { from: "CPU Core", to: "ALU", kind: "instantiates" },
  { from: "CPU Core", to: "Register File", kind: "instantiates" },
  { from: "CPU Core", to: "Branch Predictor", kind: "instantiates" },
  { from: "CPU Core", to: "Scheduler", kind: "instantiates" },
  { from: "Cache System", to: "L1 Instruction Cache", kind: "instantiates" },
  { from: "Cache System", to: "L1 Data Cache", kind: "instantiates" },
  { from: "Cache System", to: "L2 Cache", kind: "instantiates" },
  { from: "Cache System", to: "Cache Controller", kind: "instantiates" },
  { from: "Interconnect", to: "AXI Crossbar", kind: "instantiates" },
  { from: "Interconnect", to: "AXI to APB Bridge", kind: "instantiates" },
  { from: "Memory Controller", to: "DDR Controller", kind: "instantiates" },
  { from: "Memory Controller", to: "SRAM Wrapper", kind: "instantiates" },
  { from: "Memory Controller", to: "ROM", kind: "instantiates" },
  { from: "Peripherals", to: "GPIO", kind: "instantiates" },
  { from: "Peripherals", to: "UART", kind: "instantiates" },
  { from: "Peripherals", to: "SPI Master", kind: "instantiates" },
  { from: "Peripherals", to: "I2C Controller", kind: "instantiates" },
  { from: "Peripherals", to: "Timer", kind: "instantiates" },
  { from: "CPU Core", to: "uvm_pkg", kind: "imports" },
  { from: "Cache Controller", to: "uvm_pkg", kind: "imports" },
  { from: "AXI Crossbar", to: "axi_if", kind: "uses" },
  { from: "Cache System", to: "AXI Crossbar", kind: "uses" },
];

const nodeFiles: Record<string, string> = {
  "CPU Core": "core/cpu_top.sv",
  "Instruction Decoder": "core/decoder.sv",
  "ALU": "core/alu.sv",
  "Register File": "core/regfile.sv",
  "Branch Predictor": "core/bp.sv",
  "Scheduler": "core/scheduler.sv",
  "Cache System": "cache/cache_top.sv",
  "L1 Instruction Cache": "cache/icache.sv",
  "L1 Data Cache": "cache/dcache.sv",
  "L2 Cache": "cache/l2cache.sv",
  "Cache Controller": "cache/cache_ctrl.sv",
  "Interconnect": "interconnect/axi_top.sv",
  "AXI Crossbar": "interconnect/axi_crossbar.sv",
  "AXI to APB Bridge": "interconnect/axi2apb.sv",
  "Memory Controller": "memory/mem_ctrl.sv",
  "DDR Controller": "memory/ddr_ctrl.sv",
  "SRAM Wrapper": "memory/sram_wrap.sv",
  "ROM": "memory/rom.sv",
  "Peripherals": "peripherals/periph_top.sv",
  "GPIO": "peripherals/gpio.sv",
  "UART": "peripherals/uart.sv",
  "SPI Master": "peripherals/spi.sv",
  "I2C Controller": "peripherals/i2c.sv",
  "Timer": "peripherals/timer.sv",
};

const nodeKinds: Record<string, string> = {
  "Aurora-172 SoC": "SoC",
  "CPU Core": "Core",
  "Instruction Decoder": "Decoder",
  "ALU": "ALU",
  "Register File": "RegFile",
  "Branch Predictor": "Predictor",
  "Scheduler": "Scheduler",
  "Cache System": "Cache",
  "L1 Instruction Cache": "ICache",
  "L1 Data Cache": "DCache",
  "L2 Cache": "L2Cache",
  "Cache Controller": "Controller",
  "Interconnect": "Bus",
  "AXI Crossbar": "Crossbar",
  "AXI to APB Bridge": "Bridge",
  "Memory Controller": "Controller",
  "DDR Controller": "DDR",
  "SRAM Wrapper": "SRAM",
  "ROM": "ROM",
  "Peripherals": "IO",
  "GPIO": "GPIO",
  "UART": "UART",
  "SPI Master": "SPI",
  "I2C Controller": "I2C",
  "Timer": "Timer",
};

// Build tree from flat dependency list
function buildTree(deps: DepEdge[]): DepNode {
  const children = new Map<string, DepNode[]>();
  const allNodes = new Set<string>();
  const hasParent = new Set<string>();

  for (const d of deps) {
    allNodes.add(d.from);
    allNodes.add(d.to);
    hasParent.add(d.to);

    if (!children.has(d.from)) children.set(d.from, []);
    children.get(d.from)!.push({
      name: d.to,
      kind: nodeKinds[d.to] || "Module",
      file: nodeFiles[d.to] || "",
      children: [],
      icon: kindIcon(nodeKinds[d.to] || ""),
      color: kindColor(nodeKinds[d.to] || ""),
    });
  }

  // Find root (node without parent)
  let root = "Aurora-172 SoC";
  for (const n of allNodes) {
    if (!hasParent.has(n)) {
      root = n;
      break;
    }
  }

  // Recursively build
  function addChildren(node: DepNode): DepNode {
    const nodeChildren = children.get(node.name) || [];
    return {
      ...node,
      children: nodeChildren.map((c) => addChildren(c)),
    };
  }

  return addChildren({
    name: root,
    kind: nodeKinds[root] || "SoC",
    file: nodeFiles[root] || "",
    children: [],
    icon: kindIcon(nodeKinds[root] || ""),
    color: kindColor(nodeKinds[root] || ""),
  });
}

// ── Individual dependency node component ──
function DepTreeNode({ node, depth, activeNode, setActiveNode }: {
  node: DepNode;
  depth: number;
  activeNode: string | null;
  setActiveNode: (n: string | null) => void;
}) {
  const [open, setOpen] = useState(true);
  const { openFile } = useEditorStore();
  const isSelected = activeNode === node.name;
  const hasChildren = node.children.length > 0;

  return (
    <div>
      <div
        className="sidebar-tree__label"
        style={{
          paddingLeft: 10 + depth * 18,
          cursor: "pointer",
          background: isSelected ? "var(--bg-active)" : "transparent",
          borderLeft: `2px solid ${isSelected ? node.color : "transparent"}`,
          borderRadius: "0 3px 3px 0",
          margin: "1px 8px 1px 4px",
          transition: "all 0.1s",
        }}
        onClick={() => {
          setActiveNode(isSelected ? null : node.name);
          if (hasChildren) setOpen(!open);
          if (node.file) openFile(node.file, node.name);
        }}
      >
        {/* Arrow */}
        <span style={{ width: 14, flexShrink: 0, display: "flex", justifyContent: "center" }}>
          {hasChildren ? (
            <ArrowDownRight
              size={10}
              style={{
                color: node.color,
                transform: open ? "rotate(0deg)" : "rotate(-90deg)",
                transition: "transform 0.15s",
              }}
            />
          ) : (
            <span style={{ width: 10, display: "inline-block" }} />
          )}
        </span>

        {/* Icon */}
        <node.icon size={12} style={{ color: node.color, flexShrink: 0 }} />

        {/* Name */}
        <span className="sidebar-tree__name" style={{ fontWeight: depth <= 1 ? 500 : 400, fontSize: 12 }}>
          {node.name}
        </span>

        {/* Kind badge */}
        <span
          style={{
            fontSize: 9,
            color: "var(--text-muted)",
            background: `${node.color}15`,
            padding: "1px 5px",
            borderRadius: 3,
            flexShrink: 0,
            marginLeft: "auto",
          }}
        >
          {node.kind}
        </span>
      </div>

      {/* Children */}
      {hasChildren && open && (
        <div>
          {node.children.map((child, i) => (
            <DepTreeNode
              key={`${child.name}-${i}`}
              node={child}
              depth={depth + 1}
              activeNode={activeNode}
              setActiveNode={setActiveNode}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Legend component ──
function DepLegend() {
  const legend = [
    { label: "Core", color: "var(--accent-blue)" },
    { label: "Cache/Memory", color: "var(--accent-green)" },
    { label: "Interconnect", color: "var(--accent-purple)" },
    { label: "Controller", color: "var(--accent-cyan)" },
    { label: "I/O", color: "var(--accent-orange)" },
    { label: "Clock", color: "var(--accent-yellow)" },
    { label: "Verification", color: "var(--accent-teal)" },
  ];

  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: "4px 12px 8px", borderBottom: "1px solid var(--border-primary)", marginBottom: 4 }}>
      {legend.map((item) => (
        <div key={item.label} style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 9, color: "var(--text-muted)" }}>
          <span style={{ width: 6, height: 6, borderRadius: "50%", background: item.color, flexShrink: 0 }} />
          {item.label}
        </div>
      ))}
    </div>
  );
}

// ── Edge counter ──
function DepEdgeBadge({ kind }: { kind: string }) {
  const count = mockDeps.filter((d) => d.kind === kind).length;
  return (
    <span style={{
      fontSize: 10,
      color: "var(--text-muted)",
      background: "var(--bg-tertiary)",
      padding: "0 6px",
      borderRadius: 3,
    }}>
      {count} {kind}
    </span>
  );
}

export default function DependencyView() {
  const [activeNode, setActiveNode] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>("all");

  const tree = buildTree(mockDeps);

  // Get all unique node names for search
  const allNodeNames = Object.keys(nodeFiles);

  const filterOptions = [
    { label: "All", value: "all" },
    { label: "Core", value: "core" },
    { label: "Cache", value: "cache" },
    { label: "I/O", value: "io" },
    { label: "Ctrl", value: "controller" },
    { label: "Bus", value: "bus" },
    { label: "Memory", value: "memory" },
  ];

  return (
    <div className="fade-in">
      {/* Header stats */}
      <div className="sidebar-section">
        <div className="sidebar-section__title">Dependency Graph</div>
        <div className="arch-stats">
          <div className="arch-stats__item">
            <span className="arch-stats__value">{allNodeNames.length}</span>
            <span className="arch-stats__label">Modules</span>
          </div>
          <div className="arch-stats__item">
            <span className="arch-stats__value">{mockDeps.length}</span>
            <span className="arch-stats__label">Edges</span>
          </div>
          <div className="arch-stats__item">
            <span className="arch-stats__value">{mockDeps.filter((d) => d.kind === "instantiates").length}</span>
            <span className="arch-stats__label">Instances</span>
          </div>
        </div>
      </div>

      {/* Filter tabs */}
      <div className="sidebar-section">
        <div className="sidebar-section__title">Filter by type</div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 3, marginTop: 4 }}>
          {filterOptions.map((opt) => (
            <button
              key={opt.value}
              onClick={() => setFilter(opt.value)}
              style={{
                padding: "2px 8px",
                fontSize: 10,
                borderRadius: 4,
                background: filter === opt.value ? "var(--accent-blue)" : "var(--bg-tertiary)",
                color: filter === opt.value ? "white" : "var(--text-tertiary)",
                border: "none",
                cursor: "pointer",
                transition: "all 0.1s",
              }}
            >
              {opt.label}
            </button>
          ))}
        </div>
      </div>

      {/* Legend */}
      <DepLegend />

      {/* Edge type badges */}
      <div style={{ display: "flex", gap: 4, padding: "4px 12px 8px" }}>
        <DepEdgeBadge kind="instantiates" />
        <DepEdgeBadge kind="imports" />
        <DepEdgeBadge kind="uses" />
      </div>

      {/* Dependency tree */}
      <div>
        <DepTreeNode
          node={tree}
          depth={0}
          activeNode={activeNode}
          setActiveNode={setActiveNode}
        />
      </div>

      {/* Active node details */}
      {activeNode && (
        <div className="fade-in" style={{
          margin: "8px 12px",
          padding: "8px",
          background: "var(--bg-tertiary)",
          borderRadius: 5,
          border: "1px solid var(--border-primary)",
        }}>
          <div style={{ fontSize: 11, fontWeight: 600, color: "var(--text-primary)", marginBottom: 4 }}>
            {activeNode}
          </div>
          <div style={{ fontSize: 10, color: "var(--text-tertiary)", fontFamily: "var(--font-mono)" }}>
            {nodeFiles[activeNode] || "—"}
          </div>
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 2 }}>
            {nodeKinds[activeNode] || "—"}
            {" · "}
            {mockDeps.filter((d) => d.from === activeNode).length} dependents
            {" · "}
            {mockDeps.filter((d) => d.to === activeNode).length} dependencies
          </div>
        </div>
      )}
    </div>
  );
}

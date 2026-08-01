import { useState, useEffect } from "react";
import { ChevronRight, Cpu, Database, GitMerge, Monitor, HardDrive, Network, Zap, Cpu as Chip, Layers as LayersIcon, RefreshCw } from "lucide-react";
import useProjectStore from "../../stores/projectStore";
import useEditorStore from "../../stores/editorStore";
import { useProjectActions } from "../../hooks/useProjectActions";

interface ArchNodeProps {
  node: { name: string; kind: string; children: any[]; file?: string; line?: number };
  depth: number;
}

function kindIcon(kind: string) {
  const k = kind.toLowerCase();
  if (k.includes("cpu") || k.includes("core") || k.includes("decoder") || k.includes("alu") || k.includes("scheduler")) return Cpu;
  if (k.includes("cache") || k.includes("memory") || k.includes("sram") || k.includes("ddr")) return Database;
  if (k.includes("interconnect") || k.includes("bus") || k.includes("axi") || k.includes("wishbone")) return GitMerge;
  if (k.includes("controller") || k.includes("ctrl")) return Zap;
  if (k.includes("io") || k.includes("gpio") || k.includes("uart") || k.includes("spi") || k.includes("i2c")) return Monitor;
  if (k.includes("clock") || k.includes("reset") || k.includes("clk")) return HardDrive;
  if (k.includes("fifo") || k.includes("buffer")) return LayersIcon;
  if (k.includes("network") || k.includes("noc") || k.includes("router")) return Network;
  return Chip;
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

function ArchNode({ node, depth }: ArchNodeProps) {
  const [open, setOpen] = useState(true);
  const { openFile } = useEditorStore();
  const hasChildren = node.children.length > 0;
  const Icon = kindIcon(node.kind);
  const color = kindColor(node.kind);

  return (
    <div>
      <div
        className="sidebar-tree__label"
        style={{
          paddingLeft: 12 + depth * 16,
          cursor: "pointer",
          borderLeft: depth > 0 ? `2px solid ${color}22` : "none",
          marginLeft: depth > 0 ? 8 : 0,
        }}
        onClick={() => {
          if (hasChildren) setOpen(!open);
          if (node.file) openFile(node.file, node.name);
        }}
      >
        {hasChildren ? (
          <ChevronRight
            size={11}
            className={`sidebar-tree__arrow ${open ? "sidebar-tree__arrow--open" : ""}`}
            style={{ color }}
          />
        ) : (
          <span style={{ width: 11, flexShrink: 0 }} />
        )}
        <Icon size={13} style={{ color, flexShrink: 0 }} />
        <span className="sidebar-tree__name" style={{ fontWeight: depth === 0 ? 600 : 400 }}>
          {node.name}
        </span>
        <span className="arch-node__kind">{node.kind}</span>
      </div>
      {hasChildren && open && (
        <div>
          {node.children.map((child, i) => (
            <ArchNode key={i} node={child} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

// Default architecture data for demo
const defaultArchitecture = {
  name: "Aurora-172",
  kind: "SoC",
  children: [
    {
      name: "CPU Core",
      kind: "Core",
      file: "core/cpu_top.sv",
      children: [
        { name: "Instruction Decoder", kind: "Decoder", file: "core/decoder.sv", children: [] },
        { name: "ALU", kind: "ALU", file: "core/alu.sv", children: [] },
        { name: "Register File", kind: "RegFile", file: "core/regfile.sv", children: [] },
        { name: "Branch Predictor", kind: "Predictor", file: "core/bp.sv", children: [] },
        { name: "Instruction Scheduler", kind: "Scheduler", file: "core/scheduler.sv", children: [] },
      ],
    },
    {
      name: "Cache System",
      kind: "Cache",
      file: "cache/cache_top.sv",
      children: [
        { name: "L1 Instruction Cache", kind: "ICache", file: "cache/icache.sv", children: [] },
        { name: "L1 Data Cache", kind: "DCache", file: "cache/dcache.sv", children: [] },
        { name: "L2 Cache", kind: "L2Cache", file: "cache/l2cache.sv", children: [] },
        { name: "Cache Controller", kind: "Controller", file: "cache/cache_ctrl.sv", children: [] },
      ],
    },
    {
      name: "Interconnect",
      kind: "AXI Bus",
      file: "interconnect/axi_top.sv",
      children: [
        { name: "AXI Crossbar", kind: "Crossbar", file: "interconnect/axi_crossbar.sv", children: [] },
        { name: "AXI to APB Bridge", kind: "Bridge", file: "interconnect/axi2apb.sv", children: [] },
      ],
    },
    {
      name: "Memory Controller",
      kind: "Controller",
      file: "memory/mem_ctrl.sv",
      children: [
        { name: "DDR Controller", kind: "DDR", file: "memory/ddr_ctrl.sv", children: [] },
        { name: "SRAM Wrapper", kind: "SRAM", file: "memory/sram_wrap.sv", children: [] },
        { name: "ROM", kind: "ROM", file: "memory/rom.sv", children: [] },
      ],
    },
    {
      name: "Peripherals",
      kind: "IO",
      file: "peripherals/periph_top.sv",
      children: [
        { name: "GPIO", kind: "GPIO", file: "peripherals/gpio.sv", children: [] },
        { name: "UART", kind: "UART", file: "peripherals/uart.sv", children: [] },
        { name: "SPI Master", kind: "SPI", file: "peripherals/spi.sv", children: [] },
        { name: "I2C Controller", kind: "I2C", file: "peripherals/i2c.sv", children: [] },
        { name: "Timer", kind: "Timer", file: "peripherals/timer.sv", children: [] },
      ],
    },
    {
      name: "Verification",
      kind: "Testbench",
      children: [
        { name: "CPU Test Suite", kind: "UVMTests", file: "verification/cpu_tb.sv", children: [] },
        { name: "Cache Coherency Check", kind: "Assertions", file: "verification/cache_check.sv", children: [] },
        { name: "AXI Protocol Checker", kind: "Checker", file: "verification/axi_protocol.sv", children: [] },
      ],
    },
  ],
};

function countNodes(node: typeof defaultArchitecture): number {
  let count = 1;
  for (const child of node.children) {
    count += countNodes(child as any);
  }
  return count;
}

export default function ArchitectureView() {
  const { architecture, modules } = useProjectStore();
  const { loadArchitecture } = useProjectActions();
  const [loading, setLoading] = useState(false);

  // Load architecture from backend when component mounts
  useEffect(() => {
    if (modules.length > 0 && !architecture) {
      setLoading(true);
      loadArchitecture().finally(() => setLoading(false));
    }
  }, [modules.length, architecture, loadArchitecture]);

  const arch = architecture || defaultArchitecture;
  const totalNodes = architecture ? countNodes(defaultArchitecture) : 0;

  return (
    <div className="fade-in">
      <div className="sidebar-section">
        <div className="sidebar-section__title" style={{ display: "flex", alignItems: "center", gap: 6 }}>
          RTL Hierarchy
          {loading && <RefreshCw size={11} style={{ animation: "spin 0.8s linear infinite" }} />}
        </div>
        <div className="arch-stats">
          <div className="arch-stats__item">
            <span className="arch-stats__value">{architecture ? arch.children.length : 6}</span>
            <span className="arch-stats__label">Top Modules</span>
          </div>
          <div className="arch-stats__item">
            <span className="arch-stats__value">{architecture ? totalNodes : 47}</span>
            <span className="arch-stats__label">Total Instances</span>
          </div>
          <div className="arch-stats__item">
            <span className="arch-stats__value">{modules.length}</span>
            <span className="arch-stats__label">Modules</span>
          </div>
        </div>
      </div>
      <ArchNode node={arch} depth={0} />
    </div>
  );
}

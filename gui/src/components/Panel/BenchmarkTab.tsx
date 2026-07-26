import { useState, useMemo } from "react";
import ReactECharts from "echarts-for-react";
import {
  BarChart3, Clock, HardDrive, Cpu, TrendingUp,
  RotateCcw, Trash2
} from "lucide-react";
import useSimulationStore, { BenchmarkRun } from "../../stores/simulationStore";

type ChartView = "time" | "memory" | "cpu" | "throughput";

const chartViews: { id: ChartView; label: string; icon: any }[] = [
  { id: "time", label: "Timing", icon: Clock },
  { id: "memory", label: "Memory", icon: HardDrive },
  { id: "cpu", label: "CPU", icon: Cpu },
  { id: "throughput", label: "Throughput", icon: TrendingUp },
];

// ── Generate mock historical data on first load ──
function generateMockHistory(): BenchmarkRun[] {
  const now = Date.now();
  const data: BenchmarkRun[] = [];
  for (let i = 0; i < 12; i++) {
    data.push({
      id: i,
      timestamp: now - (12 - i) * 60000,
      parseTime: 0.28 + Math.random() * 0.12,
      elabTime: 1.1 + Math.random() * 0.4,
      simTime: 3.2 + Math.random() * 1.5,
      memoryMB: 128 + Math.random() * 64,
      cpuPercent: 35 + Math.random() * 25,
      modulesCount: Math.floor(42 + Math.random() * 10),
      signalsCount: Math.floor(280 + Math.random() * 60),
      throughputFiles: 180 + Math.random() * 120,
    });
  }
  return data;
}

function formatTime(ms: number): string {
  if (ms < 0.001) return `${(ms * 1000).toFixed(1)}µs`;
  if (ms < 1) return `${(ms * 1000).toFixed(0)}µs`;
  if (ms < 1000) return `${ms.toFixed(2)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

export default function BenchmarkTab() {
  const { compileResult, benchmarkHistory, clearBenchmarkHistory } = useSimulationStore();
  const [chartView, setChartView] = useState<ChartView>("time");
  const [showAllRuns, setShowAllRuns] = useState(false);

  // Initialize with mock data if empty
  const history = useMemo(() => {
    if (benchmarkHistory.length === 0) {
      return generateMockHistory();
    }
    return benchmarkHistory;
  }, [benchmarkHistory]);

  // ── Line chart color palette ──
  const COLORS = {
    parse: "#3b82f6",
    elab: "#06b6d4",
    sim: "#22c55e",
    memory: "#a855f7",
    cpu: "#f97316",
    throughput: "#14b8a6",
  };

  // ── Build chart options based on active view ──
  const chartOption = useMemo(() => {
    const labels = history.map((r) => {
      const d = new Date(r.timestamp);
      return `${d.getHours().toString().padStart(2, "0")}:${d.getMinutes().toString().padStart(2, "0")}`;
    });

    const baseGrid = { left: 55, right: 20, top: 35, bottom: 30 };
    const baseXAxis = {
      type: "category" as const,
      data: labels,
      axisLabel: { color: "#71717a", fontSize: 10 },
      axisLine: { lineStyle: { color: "#2e2f34" } },
      axisTick: { alignWithLabel: true },
    };
    const baseTooltip = {
      trigger: "axis" as const,
      backgroundColor: "#222327",
      borderColor: "#2e2f34",
      textStyle: { color: "#e4e4e7", fontSize: 12 },
    };

    if (chartView === "time") {
      return {
        tooltip: baseTooltip,
        legend: {
          data: ["Parse", "Elab", "Sim"],
          textStyle: { color: "#a1a1aa", fontSize: 11 },
          top: 0,
          right: 0,
        },
        grid: baseGrid,
        xAxis: baseXAxis,
        yAxis: {
          type: "value" as const,
          name: "Time (ms)",
          nameTextStyle: { color: "#71717a", fontSize: 10 },
          axisLabel: { color: "#71717a", fontSize: 10 },
          splitLine: { lineStyle: { color: "#2e2f3433" } },
        },
        series: [
          {
            name: "Parse",
            type: "line" as const,
            smooth: true,
            symbol: "circle",
            symbolSize: 5,
            data: history.map((r) => r.parseTime),
            lineStyle: { color: COLORS.parse, width: 2 },
            itemStyle: { color: COLORS.parse },
            areaStyle: { color: { type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: "#3b82f644" }, { offset: 1, color: "#3b82f600" }] } },
          },
          {
            name: "Elab",
            type: "line" as const,
            smooth: true,
            symbol: "circle",
            symbolSize: 5,
            data: history.map((r) => r.elabTime),
            lineStyle: { color: COLORS.elab, width: 2 },
            itemStyle: { color: COLORS.elab },
            areaStyle: { color: { type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: "#06b6d444" }, { offset: 1, color: "#06b6d400" }] } },
          },
          {
            name: "Sim",
            type: "line" as const,
            smooth: true,
            symbol: "circle",
            symbolSize: 5,
            data: history.map((r) => r.simTime),
            lineStyle: { color: COLORS.sim, width: 2 },
            itemStyle: { color: COLORS.sim },
            areaStyle: { color: { type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: "#22c55e44" }, { offset: 1, color: "#22c55e00" }] } },
          },
        ],
      };
    }

    if (chartView === "memory") {
      return {
        tooltip: baseTooltip,
        grid: baseGrid,
        xAxis: baseXAxis,
        yAxis: {
          type: "value" as const,
          name: "MB",
          nameTextStyle: { color: "#71717a", fontSize: 10 },
          axisLabel: { color: "#71717a", fontSize: 10 },
          splitLine: { lineStyle: { color: "#2e2f3433" } },
        },
        series: [{
          name: "Memory",
          type: "line" as const,
          smooth: true,
          symbol: "diamond",
          symbolSize: 6,
          data: history.map((r) => r.memoryMB),
          lineStyle: { color: COLORS.memory, width: 2 },
          itemStyle: { color: COLORS.memory },
          areaStyle: { color: { type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: "#a855f744" }, { offset: 1, color: "#a855f700" }] } },
        }],
      };
    }

    if (chartView === "cpu") {
      return {
        tooltip: baseTooltip,
        grid: baseGrid,
        xAxis: baseXAxis,
        yAxis: {
          type: "value" as const,
          name: "%",
          nameTextStyle: { color: "#71717a", fontSize: 10 },
          axisLabel: { color: "#71717a", fontSize: 10 },
          splitLine: { lineStyle: { color: "#2e2f3433" } },
          max: 100,
        },
        series: [{
          name: "CPU",
          type: "line" as const,
          smooth: true,
          symbol: "triangle",
          symbolSize: 6,
          data: history.map((r) => r.cpuPercent),
          lineStyle: { color: COLORS.cpu, width: 2 },
          itemStyle: { color: COLORS.cpu },
          areaStyle: { color: { type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: "#f9731644" }, { offset: 1, color: "#f9731600" }] } },
        }],
      };
    }

    // throughput
    return {
      tooltip: baseTooltip,
      grid: baseGrid,
      xAxis: baseXAxis,
      yAxis: {
        type: "value" as const,
        name: "files/s",
        nameTextStyle: { color: "#71717a", fontSize: 10 },
        axisLabel: { color: "#71717a", fontSize: 10 },
        splitLine: { lineStyle: { color: "#2e2f3433" } },
      },
      series: [{
        name: "Throughput",
        type: "line" as const,
        smooth: true,
        symbol: "rect",
        symbolSize: 6,
        data: history.map((r) => r.throughputFiles),
        lineStyle: { color: COLORS.throughput, width: 2 },
        itemStyle: { color: COLORS.throughput },
        areaStyle: { color: { type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: "#14b8a644" }, { offset: 1, color: "#14b8a600" }] } },
      }],
    };
  }, [history, chartView]);

  // ── Latest values summary ──
  const latest = history.length > 0 ? history[history.length - 1] : null;

  const displayRuns = showAllRuns ? history : history.slice(-8);

  return (
    <div className="fade-in">
      {/* ── Summary metrics row ── */}
      <div className="metrics-grid">
        <div className="metric-card">
          <div className="metric-card__label">Parse Time</div>
          <div className="metric-card__value" style={{ color: COLORS.parse }}>
            {latest ? formatTime(latest.parseTime) : "--"}
          </div>
          <div className="metric-card__sub">
            {latest ? `${latest.modulesCount} modules` : ""}
          </div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">Elab Time</div>
          <div className="metric-card__value" style={{ color: COLORS.elab }}>
            {latest ? formatTime(latest.elabTime) : "--"}
          </div>
          <div className="metric-card__sub">
            {latest ? `${latest.signalsCount} signals` : ""}
          </div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">Sim Time</div>
          <div className="metric-card__value" style={{ color: COLORS.sim }}>
            {latest ? formatTime(latest.simTime) : "--"}
          </div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">Memory</div>
          <div className="metric-card__value" style={{ color: COLORS.memory }}>
            {latest ? `${latest.memoryMB.toFixed(0)}` : "--"}
          </div>
          <div className="metric-card__sub">MB</div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">CPU</div>
          <div className="metric-card__value" style={{ color: COLORS.cpu }}>
            {latest ? `${latest.cpuPercent.toFixed(0)}%` : "--"}
          </div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">Throughput</div>
          <div className="metric-card__value" style={{ color: COLORS.throughput }}>
            {latest ? `${latest.throughputFiles.toFixed(0)}` : "--"}
          </div>
          <div className="metric-card__sub">files/s</div>
        </div>
      </div>

      {/* ── Chart view tabs ── */}
      <div style={{ display: "flex", gap: 4, padding: "4px 16px 0", alignItems: "center" }}>
        <div style={{ display: "flex", gap: 2, background: "var(--bg-tertiary)", borderRadius: 5, padding: 2 }}>
          {chartViews.map((v) => (
            <button
              key={v.id}
              onClick={() => setChartView(v.id)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 4,
                padding: "4px 10px",
                fontSize: 10,
                borderRadius: 4,
                background: chartView === v.id ? "var(--bg-secondary)" : "transparent",
                color: chartView === v.id ? "var(--accent-blue)" : "var(--text-tertiary)",
                border: chartView === v.id ? "1px solid var(--border-secondary)" : "1px solid transparent",
                cursor: "pointer",
                transition: "all 0.1s",
              }}
            >
              <v.icon size={11} />
              {v.label}
            </button>
          ))}
        </div>
        <div style={{ flex: 1 }} />
        <button
          onClick={() => setShowAllRuns(!showAllRuns)}
          style={{
            fontSize: 10,
            color: "var(--text-tertiary)",
            padding: "3px 8px",
            borderRadius: 4,
            cursor: "pointer",
            display: "flex",
            alignItems: "center",
            gap: 4,
          }}
        >
          <RotateCcw size={11} />
          {showAllRuns ? "Recent 8" : `All ${history.length}`}
        </button>
      </div>

      {/* ── Chart ── */}
      <div className="chart-container">
        <ReactECharts option={chartOption} style={{ height: "100%", width: "100%" }} />
      </div>

      {/* ── Run history table ── */}
      <div style={{ padding: "0 16px 8px" }}>
        <div className="panel-section__title" style={{ display: "flex", alignItems: "center", gap: 8 }}>
          Run History
          <button
            onClick={() => {
              if (confirm("Clear all benchmark history?")) clearBenchmarkHistory();
            }}
            style={{ display: "inline-flex", alignItems: "center", gap: 3, color: "var(--text-muted)", fontSize: 9, cursor: "pointer" }}
          >
            <Trash2 size={10} /> clear
          </button>
        </div>
        <div style={{
          display: "grid",
          gridTemplateColumns: "auto 1fr 1fr 1fr 1fr 1fr 1fr auto",
          gap: 4,
          fontSize: 10,
          fontFamily: "var(--font-mono)",
          color: "var(--text-tertiary)",
        }}>
          <span style={{ color: "var(--text-muted)", fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>#</span>
          <span style={{ color: "var(--text-muted)", fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>Time</span>
          <span style={{ color: COLORS.parse, fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>Parse</span>
          <span style={{ color: COLORS.elab, fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>Elab</span>
          <span style={{ color: COLORS.sim, fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>Sim</span>
          <span style={{ color: COLORS.memory, fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>Mem</span>
          <span style={{ color: COLORS.cpu, fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>CPU</span>
          <span style={{ color: "var(--text-muted)", fontWeight: 600, textTransform: "uppercase", fontSize: 9 }}>Files</span>
          {displayRuns.map((r) => (
            <>
              <span style={{ color: "var(--text-muted)" }}>{r.id}</span>
              <span>{new Date(r.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
              <span>{r.parseTime.toFixed(2)}ms</span>
              <span>{r.elabTime.toFixed(2)}ms</span>
              <span>{r.simTime.toFixed(2)}ms</span>
              <span>{r.memoryMB.toFixed(0)}MB</span>
              <span>{r.cpuPercent.toFixed(0)}%</span>
              <span>{r.throughputFiles.toFixed(0)}</span>
            </>
          ))}
        </div>
      </div>
    </div>
  );
}

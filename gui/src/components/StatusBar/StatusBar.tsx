import { useState, useEffect } from "react";
import { Activity, Layers, HardDrive, Cpu, Gauge, Zap } from "lucide-react";
import useSimulationStore from "../../stores/simulationStore";
import useProjectStore from "../../stores/projectStore";
import "./StatusBar.scss";

// ── Resource meter with animated transitions ──
function ResourceMeter({ label, value, max, color, icon }: {
  label: string;
  value: number;
  max: number;
  color: string;
  icon?: any;
}) {
  const pct = Math.min((value / max) * 100, 100);
  const Icon = icon;

  // Determine severity color
  const severity =
    pct > 90 ? "var(--accent-red)" :
    pct > 70 ? "var(--accent-yellow)" :
    color;

  return (
    <div
      className="statusbar__resource"
      title={`${label}: ${value}${label === "CPU" ? "%" : label === "RAM" ? " MB" : ""} / ${max}${label === "RAM" ? " MB" : ""}`}
    >
      {Icon && <Icon size={10} style={{ color: severity, flexShrink: 0 }} />}
      <span className="statusbar__resource-label">{label}</span>
      <div className="statusbar__resource-bar">
        <div
          className="statusbar__resource-fill"
          style={{
            width: `${pct}%`,
            background: severity,
            transition: "width 0.8s cubic-bezier(0.4, 0, 0.2, 1), background 0.4s ease",
          }}
        />
      </div>
    </div>
  );
}

// ── Generate simulated real-time metrics ──
function createMetricSimulator() {
  let cpu = 35;
  let mem = 1280;
  let threads = 64;
  let queue = 12;
  let filesPerSec = 200;

  return () => ({
    cpuPercent: Math.max(5, Math.min(98, cpu + (Math.random() - 0.5) * 12)),
    memoryMB: Math.max(256, Math.min(8192, mem + (Math.random() - 0.5) * 64)),
    threads: Math.max(8, Math.min(256, threads + Math.floor((Math.random() - 0.5) * 4))),
    queueDepth: Math.max(0, Math.min(200, queue + Math.floor((Math.random() - 0.5) * 6))),
    filesPerSec: Math.max(50, Math.min(1000, filesPerSec + (Math.random() - 0.5) * 30)),
  });
}

export default function StatusBar() {
  const { compileResult, isRunning, resourceMetrics, updateResourceMetrics } = useSimulationStore();
  const { modules, diagnostics, isLoading } = useProjectStore();
  const [time, setTime] = useState("--:--:--");
  const [showTooltip, setShowTooltip] = useState(false);

  const errors = diagnostics.filter((d) => d.level === "error").length;
  const warnings = diagnostics.filter((d) => d.level === "warning").length;

  // ── Real-time resource simulation ──
  useEffect(() => {
    const sim = createMetricSimulator();
    const tick = () => {
      const m = sim();
      updateResourceMetrics(m);
    };
    tick();
    const id = setInterval(tick, isRunning ? 800 : 2000);
    return () => clearInterval(id);
  }, [isRunning, updateResourceMetrics]);

  // ── Clock ──
  useEffect(() => {
    const tick = () => setTime(new Date().toLocaleTimeString());
    tick();
    const id = setInterval(tick, 10000);
    return () => clearInterval(id);
  }, []);

  const { cpuPercent, memoryMB, threads, queueDepth, filesPerSec } = resourceMetrics;

  return (
    <footer className="statusbar">
      {/* ── Left: compile status ── */}
      <div className="statusbar__left">
        {isLoading ? (
          <span className="statusbar__item statusbar__item--info">
            <span className="statusbar__dot" style={{ animation: "pulse-dot 1s ease-in-out infinite" }} />
            Loading...
          </span>
        ) : compileResult?.success ? (
          <span className="statusbar__item statusbar__item--ok">
            <span className="statusbar__dot" /> Compiled — {modules.length} modules
          </span>
        ) : compileResult ? (
          <span className="statusbar__item statusbar__item--err">
            <span className="statusbar__dot" /> Compile failed — {compileResult.errors.length} errors
          </span>
        ) : (
          <span className="statusbar__item">
            <span className="statusbar__dot" /> No project loaded
          </span>
        )}
      </div>

      {/* ── Center: Resource Monitor ── */}
      <div
        className="statusbar__center"
        onMouseEnter={() => setShowTooltip(true)}
        onMouseLeave={() => setShowTooltip(false)}
      >
        <ResourceMeter label="CPU" value={Math.round(cpuPercent)} max={100} color="var(--accent-blue)" icon={Cpu} />
        <ResourceMeter label="RAM" value={Math.round(memoryMB)} max={16 * 1024} color="var(--accent-green)" icon={HardDrive} />
        <div className="statusbar__sep-v" />
        <span className="statusbar__item" title="Active threads">
          <Layers size={11} />
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 10 }}>
            {threads}
          </span>
        </span>
        <span className="statusbar__item" title="Queue depth">
          <Activity size={11} />
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 10 }}>
            {queueDepth}
          </span>
        </span>
        <span className="statusbar__item" title="Throughput (files/sec)">
          <Zap size={11} />
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 10 }}>
            {filesPerSec.toFixed(0)}
          </span>
        </span>

        {/* ── Tooltip with detailed metrics ── */}
        {showTooltip && (
          <div className="statusbar__tooltip fade-in">
            <div className="statusbar__tooltip-row">
              <Cpu size={12} />
              <span>CPU</span>
              <span className="statusbar__tooltip-val">{cpuPercent.toFixed(1)}%</span>
            </div>
            <div className="statusbar__tooltip-row">
              <HardDrive size={12} />
              <span>Memory</span>
              <span className="statusbar__tooltip-val">{(memoryMB / 1024).toFixed(1)} GB</span>
            </div>
            <div className="statusbar__tooltip-row">
              <Layers size={12} />
              <span>Threads</span>
              <span className="statusbar__tooltip-val">{threads}</span>
            </div>
            <div className="statusbar__tooltip-row">
              <Activity size={12} />
              <span>Queue</span>
              <span className="statusbar__tooltip-val">{queueDepth}</span>
            </div>
            <div className="statusbar__tooltip-row">
              <Gauge size={12} />
              <span>Throughput</span>
              <span className="statusbar__tooltip-val">{filesPerSec.toFixed(0)} f/s</span>
            </div>
            <div className="statusbar__tooltip-divider" />
            <div className="statusbar__tooltip-row" style={{ color: "var(--text-muted)", fontSize: 9 }}>
              <Zap size={10} />
              <span>Simulation</span>
              <span className="statusbar__tooltip-val">
                {isRunning ? (
                  <span style={{ color: "var(--accent-green)" }}>Running</span>
                ) : (
                  <span style={{ color: "var(--text-muted)" }}>Idle</span>
                )}
              </span>
            </div>
            {/* Real compile stats from backend */}
            {compileResult?.success && (
              <>
                <div className="statusbar__tooltip-divider" />
                <div className="statusbar__tooltip-row" style={{ color: "var(--accent-blue)", fontSize: 9 }}>
                  <Zap size={10} />
                  <span>Compile</span>
                  <span className="statusbar__tooltip-val">
                    {compileResult.parseTime.toFixed(2)}ms parse
                  </span>
                </div>
              </>
            )}
          </div>
        )}
      </div>

      {/* ── Right: diagnostics + metadata ── */}
      <div className="statusbar__right">
        {isLoading && (
          <span className="statusbar__item statusbar__item--info">
            Working...
          </span>
        )}
        {isRunning && (
          <span className="statusbar__item statusbar__item--ok">
            <span className="statusbar__pulse-dot" /> Running
          </span>
        )}
        {errors > 0 && (
          <span className="statusbar__item statusbar__item--err">
            {errors} error{errors > 1 ? "s" : ""}
          </span>
        )}
        {warnings > 0 && (
          <span className="statusbar__item statusbar__item--warn">
            {warnings} warning{warnings > 1 ? "s" : ""}
          </span>
        )}
        <span className="statusbar__item statusbar__item--info">UTF-8</span>
        <span className="statusbar__item">SystemVerilog</span>
        <span className="statusbar__separator" />
        <span className="statusbar__item statusbar__item--time">{time}</span>
      </div>
    </footer>
  );
}

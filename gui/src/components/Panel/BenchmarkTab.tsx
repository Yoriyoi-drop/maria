import ReactECharts from "echarts-for-react";
import useSimulationStore from "../../stores/simulationStore";

export default function BenchmarkTab() {
  const { compileResult } = useSimulationStore();

  const chartOption = {
    tooltip: { trigger: "axis" as const },
    grid: { left: 50, right: 20, top: 30, bottom: 30 },
    xAxis: { type: "category" as const, data: ["Parse", "Elab", "Sim"], axisLabel: { color: "#71717a" } },
    yAxis: { type: "value" as const, name: "Time (ms)", nameTextStyle: { color: "#71717a" }, axisLabel: { color: "#71717a" } },
    series: [{
      type: "bar" as const,
      data: [
        { value: compileResult?.parseTime || 0, itemStyle: { color: "#3b82f6" } },
        { value: compileResult?.elabTime || 0, itemStyle: { color: "#06b6d4" } },
        { value: 0, itemStyle: { color: "#22c55e" } },
      ],
      barWidth: "40%",
    }],
  };

  return (
    <div>
      <div className="metrics-grid">
        <div className="metric-card">
          <div className="metric-card__label">Parse Time</div>
          <div className="metric-card__value">{compileResult?.parseTime.toFixed(2) || "--"}ms</div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">Elab Time</div>
          <div className="metric-card__value">{compileResult?.elabTime.toFixed(2) || "--"}ms</div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">Modules</div>
          <div className="metric-card__value">{compileResult?.success ? "OK" : "--"}</div>
        </div>
        <div className="metric-card">
          <div className="metric-card__label">Memory</div>
          <div className="metric-card__value">--</div>
          <div className="metric-card__sub">MB</div>
        </div>
      </div>
      <div className="chart-container">
        <ReactECharts option={chartOption} style={{ height: "100%" }} />
      </div>
    </div>
  );
}
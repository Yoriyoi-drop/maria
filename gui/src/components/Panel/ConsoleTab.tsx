import useSimulationStore from "../../stores/simulationStore";

const mockLogs = [
  { time: "[0]", msg: "[MARIA] Starting simulation engine..." },
  { time: "[0]", msg: "[MARIA] Elaboration complete. 47 modules. 312 signals." },
  { time: "[0]", msg: "[SIM]   reset_n = 0" },
  { time: "[5]", msg: "[SIM]   reset_n = 1" },
  { time: "[10]", msg: "[SIM]   clk = 1" },
  { time: "[15]", msg: "[SIM]   clk = 0" },
  { time: "[20]", msg: "[SIM]   clk = 1" },
  { time: "[100]", msg: "[COV]   Statement coverage: 97.3%" },
  { time: "[200]", msg: "[SIM]   Testbench complete." },
];

export default function ConsoleTab() {
  return (
    <div style={{ fontFamily: "var(--font-mono)", fontSize: 12, padding: "4px 16px" }}>
      {mockLogs.map((log, i) => (
        <div key={i} style={{ display: "flex", gap: 8, lineHeight: 1.8 }}>
          <span style={{ color: "var(--text-muted)", flexShrink: 0 }}>{log.time}</span>
          <span style={{ color: log.msg.includes("MARIA") ? "var(--accent-blue)" : "var(--text-secondary)" }}>
            {log.msg}
          </span>
        </div>
      ))}
    </div>
  );
}
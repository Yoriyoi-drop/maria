import { useState } from "react";
import { Terminal } from "lucide-react";

export default function TerminalTab() {
  const [input, setInput] = useState("");
  const [history, setHistory] = useState<string[]>([
    "$ cd project/aurora-172",
    "$ ls",
    "core/  gpu/  verification/  memory/  packages/",
    "$ cargo run -- test/tb_top.sv -T 1000",
    "[MARIA] Starting simulation...",
  ]);

  return (
    <div style={{ fontFamily: "var(--font-mono)", fontSize: 12, padding: "4px 0" }}>
      {history.map((line, i) => (
        <div key={i} style={{ padding: "0 16px", lineHeight: 1.8 }}>
          {line.startsWith("$") ? (
            <span>
              <span style={{ color: "var(--accent-green)" }}>$ </span>
              <span style={{ color: "var(--text-secondary)" }}>{line.slice(2)}</span>
            </span>
          ) : (
            <span style={{ color: "var(--text-tertiary)" }}>{line}</span>
          )}
        </div>
      ))}
      <div style={{ display: "flex", alignItems: "center", gap: 4, padding: "0 16px" }}>
        <span style={{ color: "var(--accent-green)" }}>$</span>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && input.trim()) {
              setHistory([...history, `$ ${input}`, `[${new Date().toLocaleTimeString()}] Command executed`]);
              setInput("");
            }
          }}
          style={{
            flex: 1,
            background: "transparent",
            border: "none",
            color: "var(--text-primary)",
            fontFamily: "inherit",
            fontSize: 12,
            outline: "none",
          }}
          placeholder="Type a command..."
        />
      </div>
    </div>
  );
}
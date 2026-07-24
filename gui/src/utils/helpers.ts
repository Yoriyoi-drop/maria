export function formatTime(ps: number): string {
  if (ps >= 1e6) return `${(ps / 1e6).toFixed(2)} ns`;
  if (ps >= 1e3) return `${(ps / 1e3).toFixed(2)} ps`;
  return `${ps.toFixed(0)} fs`;
}

export function binToHex(bin: string): string {
  const padded = bin.padLength(Math.ceil(bin.length / 4) * 4, "0");
  let hex = "";
  for (let i = 0; i < padded.length; i += 4) {
    const nibble = parseInt(padded.slice(i, i + 4), 2);
    hex += nibble.toString(16);
  }
  return hex.toUpperCase();
}

export function signalColor(name: string): string {
  const lower = name.toLowerCase();
  if (lower.includes("clk") || lower.includes("clock")) return "var(--accent-yellow)";
  if (lower.includes("rst") || lower.includes("reset")) return "var(--accent-red)";
  return "var(--text-primary)";
}

export function moduleKindColor(kind: string): string {
  switch (kind) {
    case "module": return "var(--accent-blue)";
    case "interface": return "var(--accent-purple)";
    case "package": return "var(--accent-cyan)";
    case "class": return "var(--accent-green)";
    case "program": return "var(--accent-orange)";
    default: return "var(--text-secondary)";
  }
}
import { AlertTriangle, Terminal, BarChart3, Target, Monitor, MessageSquare } from "lucide-react";
import useLayoutStore from "../../stores/layoutStore";
import ProblemsTab from "./ProblemsTab";
import ConsoleTab from "./ConsoleTab";
import BenchmarkTab from "./BenchmarkTab";
import CoverageTab from "./CoverageTab";
import TerminalTab from "./TerminalTab";
import LspTab from "./LspTab";
import "./BottomPanel.scss";

const tabs = [
  { id: "problems" as const, icon: AlertTriangle, label: "Problems" },
  { id: "console" as const, icon: Terminal, label: "Console" },
  { id: "benchmark" as const, icon: BarChart3, label: "Benchmark" },
  { id: "coverage" as const, icon: Target, label: "Coverage" },
  { id: "terminal" as const, icon: Monitor, label: "Terminal" },
  { id: "lsp" as const, icon: MessageSquare, label: "LSP" },
];

export default function BottomPanel() {
  const { bottomTab, setBottomTab } = useLayoutStore();

  return (
    <div className="bottom-panel">
      <div className="bottom-panel__tabs">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            className={`bottom-panel__tab ${bottomTab === tab.id ? "bottom-panel__tab--active" : ""}`}
            onClick={() => setBottomTab(tab.id)}
          >
            <tab.icon size={13} />
            <span>{tab.label}</span>
          </button>
        ))}
      </div>
      <div className="bottom-panel__content">
        {bottomTab === "problems" && <ProblemsTab />}
        {bottomTab === "console" && <ConsoleTab />}
        {bottomTab === "benchmark" && <BenchmarkTab />}
        {bottomTab === "coverage" && <CoverageTab />}
        {bottomTab === "terminal" && <TerminalTab />}
        {bottomTab === "lsp" && <LspTab />}
      </div>
    </div>
  );
}
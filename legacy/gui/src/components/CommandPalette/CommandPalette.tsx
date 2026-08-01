import { useEffect, useState, useRef, useCallback, useMemo } from "react";
import {
  Play, Square, Bug, RotateCw, FileSearch,
  BarChart3, Target, FileCode, GitBranch,
  Layers, Zap, Search, Settings, Cpu,
  Box, BookOpen
} from "lucide-react";
import useSimulationStore from "../../stores/simulationStore";
import useLayoutStore from "../../stores/layoutStore";
import "./CommandPalette.scss";

interface Command {
  id: string;
  label: string;
  description: string;
  icon: any;
  shortcut?: string;
  category: string;
  action: () => void;
}

export default function CommandPalette({ onClose }: { onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const { isRunning, setRunning } = useSimulationStore();
  const { setSidebarTab, setBottomTab } = useLayoutStore();

  const commands: Command[] = useMemo(
    () => [
      { id: "compile", label: "Compile Project", description: "Compile all SystemVerilog sources", icon: FileSearch, shortcut: "F7", category: "Build", action: () => {} },
      { id: "run", label: "Run Simulation", description: "Start simulation up to max time", icon: Play, shortcut: "F5", category: "Build", action: () => setRunning(true) },
      { id: "stop", label: "Stop Simulation", description: "Halt running simulation", icon: Square, shortcut: "F5", category: "Build", action: () => setRunning(false) },
      { id: "restart", label: "Restart Simulation", description: "Reset and reload design", icon: RotateCw, shortcut: "Shift+F5", category: "Build", action: () => {} },
      { id: "debug", label: "Toggle Debug Mode", description: "Enable/disable step-through debugging", icon: Bug, shortcut: "F6", category: "Build", action: () => {} },
      { id: "nav-project", label: "Show Project Files", description: "Open file explorer in sidebar", icon: Box, category: "Navigate", action: () => { setSidebarTab("project"); onClose(); } },
      { id: "nav-arch", label: "Show Architecture", description: "Display RTL hierarchy tree", icon: Layers, category: "Navigate", action: () => { setSidebarTab("architecture"); onClose(); } },
      { id: "nav-symbols", label: "Show Symbols", description: "Browse modules, interfaces, packages", icon: GitBranch, category: "Navigate", action: () => { setSidebarTab("symbols"); onClose(); } },
      { id: "nav-deps", label: "Show Dependencies", description: "Visual module dependency graph", icon: FileCode, category: "Navigate", action: () => { setSidebarTab("dependencies"); onClose(); } },
      { id: "nav-search", label: "Search Project", description: "Find signals, modules, parameters", icon: Search, shortcut: "Ctrl+Shift+F", category: "Navigate", action: () => { setSidebarTab("search"); onClose(); } },
      { id: "panel-problems", label: "Show Problems", description: "View errors, warnings, and hints", icon: Zap, category: "Panel", action: () => { setBottomTab("problems"); onClose(); } },
      { id: "panel-console", label: "Show Console", description: "View simulation log output", icon: Play, category: "Panel", action: () => { setBottomTab("console"); onClose(); } },
      { id: "panel-benchmark", label: "Show Benchmark", description: "Performance metrics and charts", icon: BarChart3, category: "Panel", action: () => { setBottomTab("benchmark"); onClose(); } },
      { id: "panel-coverage", label: "Show Coverage", description: "Statement, branch, toggle, FSM coverage", icon: Target, category: "Panel", action: () => { setBottomTab("coverage"); onClose(); } },
      { id: "panel-lsp", label: "Show Language Server", description: "LSP connection and diagnostics status", icon: Cpu, category: "Panel", action: () => { setBottomTab("lsp"); onClose(); } },
      { id: "lint", label: "Run Lint", description: "Check coding style and common errors", icon: FileCode, category: "Tools", action: () => {} },
      { id: "benchmark", label: "Run Benchmark Suite", description: "Measure parse, elab, and sim performance", icon: BarChart3, category: "Tools", action: () => { setBottomTab("benchmark"); onClose(); } },
      { id: "gen-module", label: "Generate Module Skeleton", description: "Create a new SystemVerilog module", icon: FileCode, category: "Tools", action: () => {} },
      { id: "gen-interface", label: "Generate Interface", description: "Create a new interface definition", icon: FileCode, category: "Tools", action: () => {} },
      { id: "export-cov", label: "Export Coverage (UCIS)", description: "Export coverage data to UCIS XML", icon: Target, category: "Tools", action: () => {} },
      { id: "settings", label: "Open Settings", description: "Configure Maria preferences", icon: Settings, shortcut: "Ctrl+,", category: "Misc", action: () => {} },
      { id: "quickstart", label: "Quick Start Guide", description: "Open the getting started documentation", icon: BookOpen, category: "Misc", action: () => {} },
    ],
    [setRunning, setSidebarTab, setBottomTab, onClose]
  );

  const filtered = useMemo(
    () =>
      query
        ? commands.filter(
            (c) =>
              c.label.toLowerCase().includes(query.toLowerCase()) ||
              c.description.toLowerCase().includes(query.toLowerCase()) ||
              c.category.toLowerCase().includes(query.toLowerCase())
          )
        : commands,
    [commands, query]
  );

  const grouped = useMemo(
    () =>
      filtered.reduce<Record<string, Command[]>>((acc, cmd) => {
        if (!acc[cmd.category]) acc[cmd.category] = [];
        acc[cmd.category].push(cmd);
        return acc;
      }, {}),
    [filtered]
  );

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  const execute = useCallback(
    (cmd: Command) => {
      cmd.action();
      onClose();
    },
    [onClose]
  );

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      }
      if (e.key === "Enter" && filtered[selectedIndex]) {
        execute(filtered[selectedIndex]);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose, filtered, selectedIndex, execute]);

  let flatIndex = 0;

  return (
    <div className="command-palette-overlay" onClick={onClose}>
      <div className="command-palette fade-in" onClick={(e) => e.stopPropagation()}>
        <div className="command-palette__header">
          <Search size={15} className="command-palette__search-icon" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Type a command..."
            className="command-palette__input"
          />
          <kbd className="command-palette__esc">ESC</kbd>
        </div>

        <div className="command-palette__results">
          {Object.entries(grouped).map(([category, cmds]) => (
            <div key={category} className="command-palette__group">
              <div className="command-palette__category">{category}</div>
              {cmds.map((cmd) => {
                const idx = flatIndex++;
                const isSelected = idx === selectedIndex;
                return (
                  <div
                    key={cmd.id}
                    className={`command-palette__item ${isSelected ? "command-palette__item--selected" : ""}`}
                    onClick={() => execute(cmd)}
                    onMouseEnter={() => setSelectedIndex(idx)}
                  >
                    <cmd.icon size={15} className="command-palette__item-icon" />
                    <div className="command-palette__item-text">
                      <span className="command-palette__item-label">{cmd.label}</span>
                      <span className="command-palette__item-desc">{cmd.description}</span>
                    </div>
                    {cmd.shortcut && <kbd className="command-palette__shortcut">{cmd.shortcut}</kbd>}
                  </div>
                );
              })}
            </div>
          ))}
          {filtered.length === 0 && (
            <div className="command-palette__empty">
              No commands match <strong>"{query}"</strong>
            </div>
          )}
        </div>

        <div className="command-palette__footer">
          <span>↑↓ Navigate</span>
          <span>↵ Execute</span>
          <span>Esc Close</span>
        </div>
      </div>
    </div>
  );
}

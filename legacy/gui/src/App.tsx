import { useEffect } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import Toolbar from "./components/Toolbar/Toolbar";
import Sidebar from "./components/Sidebar/Sidebar";
import EditorArea from "./components/Editor/EditorArea";
import BottomPanel from "./components/Panel/BottomPanel";
import StatusBar from "./components/StatusBar/StatusBar";
import CommandPalette from "./components/CommandPalette/CommandPalette";
import useLayoutStore from "./stores/layoutStore";
import useSimulationStore from "./stores/simulationStore";
import useProjectStore from "./stores/projectStore";
import { useProjectActions } from "./hooks/useProjectActions";
import "./styles/app.scss";

export default function App() {
  const { sidebarWidth, bottomHeight, showCommandPalette, toggleCommandPalette, closeCommandPalette } =
    useLayoutStore();
  const { isRunning } = useSimulationStore();
  const { projectName, isLoading } = useProjectStore();
  const { openProject, compileProject, runSimulation, stopSimulation } = useProjectActions();

  // Global keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Ignore if in input/textarea
      const tag = (e.target as HTMLElement)?.tagName;
      const isInput = tag === "INPUT" || tag === "TEXTAREA";

      // Ctrl+Shift+P → Command Palette (always works)
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === "p") {
        e.preventDefault();
        toggleCommandPalette();
        return;
      }

      // Escape → close command palette
      if (e.key === "Escape" && showCommandPalette) {
        e.preventDefault();
        closeCommandPalette();
        return;
      }

      // Don't process other shortcuts when in input fields
      if (isInput) return;

      // F5 → Run / Stop Simulation
      if (e.key === "F5") {
        e.preventDefault();
        if (!projectName || isLoading) return;
        if (isRunning) {
          stopSimulation();
        } else {
          runSimulation();
        }
        return;
      }

      // F7 → Compile
      if (e.key === "F7") {
        e.preventDefault();
        if (!projectName || isLoading) return;
        compileProject();
        return;
      }

      // Ctrl+O → Open Project
      if ((e.ctrlKey || e.metaKey) && e.key === "o") {
        e.preventDefault();
        if (isLoading) return;
        openProject("");
        return;
      }

      // Ctrl+` → Toggle Bottom Panel
      if ((e.ctrlKey || e.metaKey) && e.key === "`") {
        e.preventDefault();
        useLayoutStore.getState().toggleBottom();
        return;
      }

      // Ctrl+B → Toggle Sidebar
      if ((e.ctrlKey || e.metaKey) && e.key === "b") {
        e.preventDefault();
        useLayoutStore.getState().toggleSidebar();
        return;
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [
    toggleCommandPalette, closeCommandPalette, showCommandPalette,
    projectName, isLoading, isRunning,
    runSimulation, stopSimulation, compileProject, openProject,
  ]);

  return (
    <div className="app">
      <Toolbar />
      <div className="app__body">
        <PanelGroup direction="horizontal" autoSaveId="main">
          <Panel defaultSize={sidebarWidth} minSize={15} maxSize={40}>
            <Sidebar />
          </Panel>
          <PanelResizeHandle className="resize-handle resize-handle--v" />
          <Panel minSize={30}>
            <PanelGroup direction="vertical" autoSaveId="editor-panel">
              <Panel minSize={20}>
                <EditorArea />
              </Panel>
              <PanelResizeHandle className="resize-handle resize-handle--h" />
              <Panel defaultSize={bottomHeight} minSize={8} maxSize={60}>
                <BottomPanel />
              </Panel>
            </PanelGroup>
          </Panel>
        </PanelGroup>
      </div>
      <StatusBar />
      {showCommandPalette && <CommandPalette onClose={closeCommandPalette} />}
    </div>
  );
}

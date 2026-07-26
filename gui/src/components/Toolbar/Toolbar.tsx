import { Play, Square, Bug, FileSearch, RotateCw, FolderOpen, Settings } from "lucide-react";
import useSimulationStore from "../../stores/simulationStore";
import useProjectStore from "../../stores/projectStore";
import { useProjectActions } from "../../hooks/useProjectActions";
import "./Toolbar.scss";

export default function Toolbar() {
  const { isRunning, setRunning } = useSimulationStore();
  const { projectName, isLoading } = useProjectStore();
  const { openProject, compileProject, runSimulation, stopSimulation } = useProjectActions();

  const handleRunToggle = () => {
    if (isRunning) {
      stopSimulation();
    } else {
      runSimulation();
    }
  };

  const handleOpenProject = async () => {
    // In Tauri, use dialog to pick a directory
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        await openProject(selected as string);
      }
    } catch {
      // Fallback: prompt for path
      const path = prompt("Enter project directory path:");
      if (path) {
        await openProject(path);
      }
    }
  };

  return (
    <header className="toolbar">
      <div className="toolbar__left">
        <span className="toolbar__brand">Maria</span>
        {projectName && (
          <>
            <span className="toolbar__sep" />
            <span className="toolbar__project">{projectName}</span>
          </>
        )}
      </div>

      <div className="toolbar__center">
        <button
          className="toolbar__btn"
          title="Open Project (Ctrl+O)"
          onClick={handleOpenProject}
          disabled={isLoading}
        >
          <FolderOpen size={16} />
        </button>
        <button
          className="toolbar__btn"
          title="Compile (F7)"
          onClick={compileProject}
          disabled={isLoading || !projectName}
        >
          <FileSearch size={16} />
        </button>
        <span className="toolbar__divider" />
        <button
          className={`toolbar__btn ${isRunning ? "toolbar__btn--active" : ""}`}
          title={isRunning ? "Stop Simulation (F5)" : "Run Simulation (F5)"}
          onClick={handleRunToggle}
          disabled={isLoading}
        >
          {isRunning ? <Square size={16} /> : <Play size={16} />}
        </button>
        <button className="toolbar__btn" title="Step (F6)" disabled={isLoading || !projectName}>
          <Bug size={16} />
        </button>
        <button className="toolbar__btn" title="Restart (Shift+F5)" disabled={isLoading || !projectName}>
          <RotateCw size={16} />
        </button>
      </div>

      <div className="toolbar__right">
        <button className="toolbar__btn" title="Settings">
          <Settings size={16} />
        </button>
      </div>
    </header>
  );
}

import { Play, Square, Bug, FileSearch, RotateCw, FolderOpen, Settings } from "lucide-react";
import useSimulationStore from "../../stores/simulationStore";
import useProjectStore from "../../stores/projectStore";
import "./Toolbar.scss";

export default function Toolbar() {
  const { isRunning, setRunning } = useSimulationStore();
  const { projectName } = useProjectStore();

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
        <button className="toolbar__btn" title="Open Project (Ctrl+O)">
          <FolderOpen size={16} />
        </button>
        <button className="toolbar__btn" title="Compile (F7)">
          <FileSearch size={16} />
        </button>
        <span className="toolbar__divider" />
        <button
          className={`toolbar__btn ${isRunning ? "toolbar__btn--active" : ""}`}
          title="Run Simulation (F5)"
          onClick={() => setRunning(!isRunning)}
        >
          {isRunning ? <Square size={16} /> : <Play size={16} />}
        </button>
        <button className="toolbar__btn" title="Step (F6)">
          <Bug size={16} />
        </button>
        <button className="toolbar__btn" title="Restart (Shift+F5)">
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
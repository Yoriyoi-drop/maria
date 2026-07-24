import useSimulationStore from "../../stores/simulationStore";
import useProjectStore from "../../stores/projectStore";
import "./StatusBar.scss";

export default function StatusBar() {
  const { compileResult } = useSimulationStore();
  const { modules, diagnostics } = useProjectStore();

  const errors = diagnostics.filter((d) => d.level === "error").length;
  const warnings = diagnostics.filter((d) => d.level === "warning").length;

  return (
    <footer className="statusbar">
      <div className="statusbar__left">
        {compileResult ? (
          compileResult.success ? (
            <span className="statusbar__item statusbar__item--ok">
              Compiled — {modules.length} modules
            </span>
          ) : (
            <span className="statusbar__item statusbar__item--err">
              Compile failed
            </span>
          )
        ) : (
          <span className="statusbar__item">No project loaded</span>
        )}
      </div>
      <div className="statusbar__right">
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
        <span className="statusbar__item">UTF-8</span>
        <span className="statusbar__item">SystemVerilog</span>
      </div>
    </footer>
  );
}
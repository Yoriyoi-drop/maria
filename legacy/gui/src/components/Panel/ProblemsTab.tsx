import { AlertTriangle, AlertCircle, Info } from "lucide-react";
import useProjectStore from "../../stores/projectStore";

export default function ProblemsTab() {
  const { diagnostics } = useProjectStore();

  if (diagnostics.length === 0) {
    return <div className="panel-empty">No problems detected</div>;
  }

  const iconMap = {
    error: AlertCircle,
    warning: AlertTriangle,
    info: Info,
  } as const;

  return (
    <div>
      {diagnostics.map((d, i) => {
        const Icon = iconMap[d.level];
        return (
          <div key={i} className="diag-item">
            <Icon size={13} className={`diag-item__icon diag-item__icon--${d.level}`} />
            <span className="diag-item__location">{d.file}:{d.line}</span>
            <span className="diag-item__msg">{d.message}</span>
          </div>
        );
      })}
    </div>
  );
}
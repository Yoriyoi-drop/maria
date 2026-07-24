import { useState } from "react";
import { ChevronRight, Cpu } from "lucide-react";
import useProjectStore from "../../stores/projectStore";
import useEditorStore from "../../stores/editorStore";

interface ArchNodeProps {
  node: { name: string; kind: string; children: any[]; file?: string; line?: number };
  depth: number;
}

function ArchNode({ node, depth }: ArchNodeProps) {
  const [open, setOpen] = useState(true);
  const { openFile } = useEditorStore();
  const hasChildren = node.children.length > 0;

  return (
    <div>
      <div
        className="sidebar-tree__label"
        style={{ paddingLeft: 12 + depth * 16, cursor: node.file ? "pointer" : "default" }}
        onClick={() => {
          if (hasChildren) setOpen(!open);
          if (node.file) openFile(node.file, node.name);
        }}
      >
        {hasChildren ? (
          <ChevronRight
            size={12}
            className={`sidebar-tree__arrow ${open ? "sidebar-tree__arrow--open" : ""}`}
          />
        ) : (
          <span style={{ width: 12, flexShrink: 0 }} />
        )}
        <Cpu size={13} className="sidebar-item__icon" />
        <span className="sidebar-tree__name">{node.name}</span>
        <span style={{ fontSize: 10, color: "var(--text-muted)" }}>{node.kind}</span>
      </div>
      {hasChildren && open && (
        <div>
          {node.children.map((child, i) => (
            <ArchNode key={i} node={child} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

export default function ArchitectureView() {
  const { architecture } = useProjectStore();

  if (!architecture) {
    return (
      <div className="sidebar-section">
        <p style={{ padding: "12px", color: "var(--text-tertiary)", fontSize: 12 }}>
          Compile a project to view architecture
        </p>
      </div>
    );
  }

  return (
    <div>
      <div className="sidebar-section">
        <div className="sidebar-section__title">RTL Hierarchy</div>
      </div>
      <ArchNode node={architecture} depth={0} />
    </div>
  );
}
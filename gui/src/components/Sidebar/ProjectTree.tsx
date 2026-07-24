import { useState } from "react";
import { File, Folder, FolderOpen, ChevronRight } from "lucide-react";
import useProjectStore from "../../stores/projectStore";
import useEditorStore from "../../stores/editorStore";

interface TreeNodeProps {
  node: { name: string; path: string; kind: "file" | "directory"; children?: any[] };
  depth: number;
}

function TreeNode({ node, depth }: TreeNodeProps) {
  const [open, setOpen] = useState(false);
  const { openFile } = useEditorStore();
  const isDir = node.kind === "directory";

  return (
    <div className="sidebar-tree__item">
      <div
        className="sidebar-tree__label"
        style={{ paddingLeft: 12 + depth * 14 }}
        onClick={() => {
          if (isDir) setOpen(!open);
          else openFile(node.path, node.name);
        }}
      >
        {isDir ? (
          <ChevronRight
            size={12}
            className={`sidebar-tree__arrow ${open ? "sidebar-tree__arrow--open" : ""}`}
          />
        ) : (
          <span className="sidebar-tree__arrow--hidden" style={{ width: 12 }} />
        )}
        {isDir ? (
          open ? (
            <FolderOpen size={14} className="sidebar-item__icon" />
          ) : (
            <Folder size={14} className="sidebar-item__icon" />
          )
        ) : (
          <File size={14} className="sidebar-item__icon" />
        )}
        <span className="sidebar-tree__name">{node.name}</span>
      </div>
      {isDir && open && node.children && (
        <div className="sidebar-tree__children">
          {node.children.map((child, i) => (
            <TreeNode key={child.path || i} node={child} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

export default function ProjectTree() {
  const { files, projectName } = useProjectStore();

  if (!projectName) {
    return (
      <div className="sidebar-section">
        <p style={{ padding: "12px", color: "var(--text-tertiary)", fontSize: 12 }}>
          Open a project to browse files
        </p>
      </div>
    );
  }

  return (
    <div>
      <div className="sidebar-section">
        <div className="sidebar-section__title">{projectName}</div>
      </div>
      {files.map((file, i) => (
        <TreeNode key={file.path || i} node={file} depth={0} />
      ))}
    </div>
  );
}
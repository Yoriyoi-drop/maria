import { useState, useCallback } from "react";
import { File, Folder, FolderOpen, ChevronRight, RefreshCw } from "lucide-react";
import useProjectStore from "../../stores/projectStore";
import useEditorStore from "../../stores/editorStore";
import { getFileTree, readFile } from "../../hooks/useMariaIPC";

interface TreeNodeProps {
  node: { name: string; path: string; kind: "file" | "directory"; children?: any[] };
  depth: number;
}

function TreeNode({ node, depth }: TreeNodeProps) {
  const [open, setOpen] = useState(false);
  const { openFile, setFileContent } = useEditorStore();
  const isDir = node.kind === "directory";

  const handleClick = async () => {
    if (isDir) {
      setOpen(!open);
    } else {
      openFile(node.path, node.name);
      // Load file content from backend
      try {
        const content = await readFile(node.path);
        setFileContent(node.path, content);
      } catch {
        // File read error — skip, editor shows placeholder
      }
    }
  };

  return (
    <div className="sidebar-tree__item">
      <div
        className="sidebar-tree__label"
        style={{ paddingLeft: 12 + depth * 14 }}
        onClick={handleClick}
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
  const { files, projectName, rootPath } = useProjectStore();
  const [refreshing, setRefreshing] = useState(false);

  const refreshFileTree = useCallback(async () => {
    if (!rootPath) return;
    setRefreshing(true);
    try {
      const treeNodes = await getFileTree(rootPath);
      useProjectStore.getState().setFiles(
        treeNodes.map((n) => ({
          name: n.name,
          path: n.path,
          kind: n.kind as "file" | "directory",
          children: n.children?.map((c: any) => ({
            name: c.name,
            path: c.path,
            kind: c.kind as "file" | "directory",
            children: c.children,
          })),
        }))
      );
    } catch {
      // Silently fail
    } finally {
      setRefreshing(false);
    }
  }, [rootPath]);

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
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <span className="sidebar-section__title" style={{ flex: 1 }}>{projectName}</span>
          <button
            onClick={refreshFileTree}
            title="Refresh file tree"
            className="sidebar-item"
            style={{ padding: "2px 4px" }}
            disabled={refreshing}
          >
            <RefreshCw size={11} style={{ animation: refreshing ? "spin 0.8s linear infinite" : "none" }} />
          </button>
        </div>
      </div>
      {files.map((file, i) => (
        <TreeNode key={file.path || i} node={file} depth={0} />
      ))}
    </div>
  );
}

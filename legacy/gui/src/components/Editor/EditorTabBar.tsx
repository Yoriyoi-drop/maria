import { X } from "lucide-react";
import useEditorStore from "../../stores/editorStore";

export default function EditorTabBar() {
  const { openFiles, activeFile, setActiveFile, closeFile } = useEditorStore();

  return (
    <div className="editor-tabs">
      {openFiles.map((f) => (
        <div
          key={f.path}
          className={`editor-tabs__tab ${activeFile === f.path ? "editor-tabs__tab--active" : ""} ${f.isDirty ? "editor-tabs__tab--dirty" : ""}`}
          onClick={() => setActiveFile(f.path)}
          onMouseDown={(e) => {
            if (e.button === 1) closeFile(f.path);
          }}
        >
          {f.name}
          <button
            className="editor-tabs__close"
            onClick={(e) => {
              e.stopPropagation();
              closeFile(f.path);
            }}
          >
            <X size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}
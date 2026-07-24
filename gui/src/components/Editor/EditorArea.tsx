import useEditorStore from "../../stores/editorStore";
import EditorTabBar from "./EditorTabBar";
import MonacoWrapper from "./MonacoWrapper";
import WelcomeScreen from "./WelcomeScreen";
import "./EditorArea.scss";

export default function EditorArea() {
  const { openFiles, activeFile } = useEditorStore();

  return (
    <div className="editor-area">
      {openFiles.length > 0 ? (
        <>
          <EditorTabBar />
          <div className="editor-area__content">
            {activeFile && <MonacoWrapper />}
          </div>
        </>
      ) : (
        <WelcomeScreen />
      )}
    </div>
  );
}
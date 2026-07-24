import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import Toolbar from "./components/Toolbar/Toolbar";
import Sidebar from "./components/Sidebar/Sidebar";
import EditorArea from "./components/Editor/EditorArea";
import BottomPanel from "./components/Panel/BottomPanel";
import StatusBar from "./components/StatusBar/StatusBar";
import useLayoutStore from "./stores/layoutStore";
import "./styles/app.scss";

export default function App() {
  const { sidebarWidth, bottomHeight, sidebarTab } = useLayoutStore();

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
    </div>
  );
}
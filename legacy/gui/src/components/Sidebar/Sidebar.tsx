import { FolderTree, Binary, Code2, GitBranch, Search } from "lucide-react";
import useLayoutStore from "../../stores/layoutStore";
import ProjectTree from "./ProjectTree";
import ArchitectureView from "./ArchitectureView";
import SymbolsView from "./SymbolsView";
import DependencyView from "./DependencyView";
import SearchView from "./SearchView";
import "./Sidebar.scss";

const tabs = [
  { id: "project" as const, icon: FolderTree, label: "Project" },
  { id: "architecture" as const, icon: Binary, label: "Architecture" },
  { id: "symbols" as const, icon: Code2, label: "Symbols" },
  { id: "dependencies" as const, icon: GitBranch, label: "Dependencies" },
  { id: "search" as const, icon: Search, label: "Search" },
];

export default function Sidebar() {
  const { sidebarTab, setSidebarTab } = useLayoutStore();

  return (
    <aside className="sidebar">
      <nav className="sidebar__tabs">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            className={`sidebar__tab ${sidebarTab === tab.id ? "sidebar__tab--active" : ""}`}
            onClick={() => setSidebarTab(tab.id)}
            title={tab.label}
          >
            <tab.icon size={16} />
          </button>
        ))}
      </nav>
      <div className="sidebar__content">
        {sidebarTab === "project" && <ProjectTree />}
        {sidebarTab === "architecture" && <ArchitectureView />}
        {sidebarTab === "symbols" && <SymbolsView />}
        {sidebarTab === "dependencies" && <DependencyView />}
        {sidebarTab === "search" && <SearchView />}
      </div>
    </aside>
  );
}
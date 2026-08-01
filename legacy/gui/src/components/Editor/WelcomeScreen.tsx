import { useState } from "react";
import { FolderOpen, FileCode, BookOpen, Clock, BarChart3, Cpu, HardDrive, Activity } from "lucide-react";
import { useProjectActions } from "../../hooks/useProjectActions";
import useProjectStore from "../../stores/projectStore";
import "./WelcomeScreen.scss";

export default function WelcomeScreen() {
  const [activeTab, setActiveTab] = useState<"recent" | "stats">("recent");
  const { openProject, compileProject, runSimulation } = useProjectActions();
  const { isLoading } = useProjectStore();

  const recentProjects = [
    { name: "Aurora-172", path: "/projects/aurora-172", date: "Today 14:32", modules: 47 },
    { name: "OpenTitan Earl Grey", path: "/projects/opentitan", date: "Yesterday 09:15", modules: 312 },
    { name: "Ibex Core", path: "/projects/ibex", date: "3 days ago", modules: 23 },
    { name: "PicoRV32", path: "/projects/picorv32", date: "Last week", modules: 5 },
  ];

  const handleOpenProject = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        await openProject(selected as string);
      }
    } catch {
      const path = prompt("Enter project directory path:");
      if (path) {
        await openProject(path);
      }
    }
  };

  const handleOpenRecentProject = async (path: string) => {
    await openProject(path);
  };

  const handleQuickStart = () => {
    window.open("https://github.com/opencode-ai/maria", "_blank");
  };

  return (
    <div className="welcome">
      <div className="welcome__header">
        <div className="welcome__brand">
          <span className="welcome__logo">M</span>
          <div>
            <h1 className="welcome__title">Maria</h1>
            <p className="welcome__subtitle">RTL Engineering Control Center</p>
          </div>
        </div>
      </div>

      {/* Quick Actions */}
      <div className="welcome__actions">
        <button
          className="welcome__btn welcome__btn--primary"
          onClick={handleOpenProject}
          disabled={isLoading}
        >
          <FolderOpen size={16} />
          Open Project
        </button>
        <button
          className="welcome__btn welcome__btn--secondary"
          onClick={compileProject}
          disabled={isLoading}
        >
          <FileCode size={16} />
          Compile Project
        </button>
        <button className="welcome__btn welcome__btn--secondary" onClick={handleQuickStart}>
          <BookOpen size={16} />
          Quick Start Guide
        </button>
      </div>

      {/* Observatory Stats */}
      <div className="welcome__observatory">
        <div className="welcome__obs-item">
          <Cpu size={15} className="welcome__obs-icon" />
          <div className="welcome__obs-info">
            <span className="welcome__obs-value">128</span>
            <span className="welcome__obs-label">Threads</span>
          </div>
        </div>
        <div className="welcome__obs-item">
          <HardDrive size={15} className="welcome__obs-icon" />
          <div className="welcome__obs-info">
            <span className="welcome__obs-value">9.2</span>
            <span className="welcome__obs-label">GB RAM</span>
          </div>
        </div>
        <div className="welcome__obs-item">
          <Activity size={15} className="welcome__obs-icon" />
          <div className="welcome__obs-info">
            <span className="welcome__obs-value">41%</span>
            <span className="welcome__obs-label">CPU</span>
          </div>
        </div>
        <div className="welcome__obs-item">
          <BarChart3 size={15} className="welcome__obs-icon" />
          <div className="welcome__obs-info">
            <span className="welcome__obs-value">0.31</span>
            <span className="welcome__obs-label">ms parse</span>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="welcome__tabs-bar">
        <button
          className={`welcome__tab ${activeTab === "recent" ? "welcome__tab--active" : ""}`}
          onClick={() => setActiveTab("recent")}
        >
          <Clock size={13} />
          Recent Projects
        </button>
        <button
          className={`welcome__tab ${activeTab === "stats" ? "welcome__tab--active" : ""}`}
          onClick={() => setActiveTab("stats")}
        >
          <BarChart3 size={13} />
          System Stats
        </button>
      </div>

      <div className="welcome__tab-content">
        {activeTab === "recent" && (
          <div className="welcome__recent">
            {recentProjects.map((p, i) => (
              <div
                key={i}
                className="welcome__project-item"
                onClick={() => handleOpenRecentProject(p.path)}
              >
                <div className="welcome__project-icon">
                  <FileCode size={14} />
                </div>
                <div className="welcome__project-info">
                  <span className="welcome__project-name">{p.name}</span>
                  <span className="welcome__project-path">{p.path}</span>
                </div>
                <div className="welcome__project-meta">
                  <span className="welcome__project-modules">{p.modules} modules</span>
                  <span className="welcome__project-date">{p.date}</span>
                </div>
              </div>
            ))}
          </div>
        )}

        {activeTab === "stats" && (
          <div className="welcome__stats-grid">
            <div className="welcome__stat-card">
              <span className="welcome__stat-label">Parse Time</span>
              <span className="welcome__stat-value">0.31 ms</span>
              <span className="welcome__stat-trend welcome__stat-trend--up">+12% vs baseline</span>
            </div>
            <div className="welcome__stat-card">
              <span className="welcome__stat-label">Elab Time</span>
              <span className="welcome__stat-value">1.24 ms</span>
              <span className="welcome__stat-trend welcome__stat-trend--up">+5% vs baseline</span>
            </div>
            <div className="welcome__stat-card">
              <span className="welcome__stat-label">Memory Usage</span>
              <span className="welcome__stat-value">9.2 GB</span>
              <span className="welcome__stat-trend welcome__stat-trend--down">-8% vs baseline</span>
            </div>
            <div className="welcome__stat-card">
              <span className="welcome__stat-label">Coverage</span>
              <span className="welcome__stat-value">97.3%</span>
              <span className="welcome__stat-trend welcome__stat-trend--up">+2.1% vs baseline</span>
            </div>
          </div>
        )}
      </div>

      {/* Shortcuts Footer */}
      <div className="welcome__shortcuts">
        <div><kbd>Ctrl+O</kbd> Open</div>
        <div><kbd>F7</kbd> Compile</div>
        <div><kbd>F5</kbd> Run</div>
        <div><kbd>Ctrl+Shift+P</kbd> Commands</div>
      </div>
    </div>
  );
}

import { FolderOpen, FileCode, BookOpen } from "lucide-react";

export default function WelcomeScreen() {
  return (
    <div className="welcome">
      <div className="welcome__title">Maria</div>
      <div className="welcome__subtitle">
        RTL Engineering Control Center
        <br />
        SystemVerilog simulator, analyzer, and observatory
      </div>
      <div className="welcome__actions">
        <button className="welcome__btn welcome__btn--primary">
          <FolderOpen size={16} />
          Open Project
        </button>
        <button className="welcome__btn welcome__btn--secondary">
          <FileCode size={16} />
          New File
        </button>
        <button className="welcome__btn welcome__btn--secondary">
          <BookOpen size={16} />
          Quick Start
        </button>
      </div>
      <div className="welcome__shortcuts">
        <div>
          <kbd>Ctrl+O</kbd> Open
        </div>
        <div>
          <kbd>F7</kbd> Compile
        </div>
        <div>
          <kbd>F5</kbd> Run
        </div>
        <div>
          <kbd>Ctrl+Shift+P</kbd> Commands
        </div>
      </div>
    </div>
  );
}
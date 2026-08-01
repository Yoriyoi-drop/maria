import { create } from "zustand";

export interface OpenFile {
  path: string;
  name: string;
  language: string;
  content?: string;
  isDirty: boolean;
}

interface EditorState {
  openFiles: OpenFile[];
  activeFile: string | null;
  diagnostics: { file: string; line: number; message: string; level: "error" | "warning" | "info" }[];
  openFile: (path: string, name: string) => void;
  closeFile: (path: string) => void;
  setActiveFile: (path: string | null) => void;
  setFileContent: (path: string, content: string) => void;
  setDiagnostics: (d: EditorState["diagnostics"]) => void;
  markDirty: (path: string) => void;
  markClean: (path: string) => void;
}

export default create<EditorState>((set) => ({
  openFiles: [],
  activeFile: null,
  diagnostics: [],
  openFile: (path, name) =>
    set((s) => {
      if (s.openFiles.find((f) => f.path === path)) return { activeFile: path };
      return {
        openFiles: [...s.openFiles, { path, name, language: "systemverilog", isDirty: false }],
        activeFile: path,
      };
    }),
  closeFile: (path) =>
    set((s) => {
      const files = s.openFiles.filter((f) => f.path !== path);
      const active = s.activeFile === path ? (files[files.length - 1]?.path ?? null) : s.activeFile;
      return { openFiles: files, activeFile: active };
    }),
  setActiveFile: (path) => set({ activeFile: path }),
  setFileContent: (path, content) =>
    set((s) => ({
      openFiles: s.openFiles.map((f) => (f.path === path ? { ...f, content, isDirty: false } : f)),
    })),
  setDiagnostics: (d) => set({ diagnostics: d }),
  markDirty: (path) =>
    set((s) => ({
      openFiles: s.openFiles.map((f) => (f.path === path ? { ...f, isDirty: true } : f)),
    })),
  markClean: (path) =>
    set((s) => ({
      openFiles: s.openFiles.map((f) => (f.path === path ? { ...f, isDirty: false } : f)),
    })),
}));
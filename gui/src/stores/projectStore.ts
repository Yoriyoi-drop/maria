import { create } from "zustand";

export interface FileNode {
  name: string;
  path: string;
  kind: "file" | "directory";
  children?: FileNode[];
}

export interface ModuleInfo {
  name: string;
  file: string;
  line: number;
  kind: "module" | "interface" | "package" | "program" | "class";
}

export interface ArchitectureNode {
  name: string;
  kind: string;
  children: ArchitectureNode[];
  file?: string;
  line?: number;
}

interface ProjectState {
  projectName: string;
  rootPath: string;
  files: FileNode[];
  modules: ModuleInfo[];
  architecture: ArchitectureNode | null;
  isLoading: boolean;
  diagnostics: { file: string; line: number; message: string; level: "error" | "warning" | "info" }[];
  setProject: (name: string, path: string) => void;
  setFiles: (files: FileNode[]) => void;
  setModules: (modules: ModuleInfo[]) => void;
  setArchitecture: (arch: ArchitectureNode) => void;
  setLoading: (v: boolean) => void;
  setDiagnostics: (d: ProjectState["diagnostics"]) => void;
}

export default create<ProjectState>((set) => ({
  projectName: "",
  rootPath: "",
  files: [],
  modules: [],
  architecture: null,
  isLoading: false,
  diagnostics: [],
  setProject: (name, path) => set({ projectName: name, rootPath: path }),
  setFiles: (files) => set({ files }),
  setModules: (modules) => set({ modules }),
  setArchitecture: (arch) => set({ architecture: arch }),
  setLoading: (v) => set({ isLoading: v }),
  setDiagnostics: (d) => set({ diagnostics: d }),
}));
import { create } from "zustand";

export type SidebarTab = "project" | "architecture" | "symbols" | "dependencies" | "search";
export type BottomTab = "problems" | "console" | "benchmark" | "coverage" | "terminal" | "lsp";

interface LayoutState {
  sidebarWidth: number;
  bottomHeight: number;
  sidebarTab: SidebarTab;
  bottomTab: BottomTab;
  showSidebar: boolean;
  showBottom: boolean;
  setSidebarWidth: (w: number) => void;
  setBottomHeight: (h: number) => void;
  setSidebarTab: (t: SidebarTab) => void;
  setBottomTab: (t: BottomTab) => void;
  toggleSidebar: () => void;
  toggleBottom: () => void;
}

export default create<LayoutState>((set) => ({
  sidebarWidth: 22,
  bottomHeight: 25,
  sidebarTab: "project",
  bottomTab: "problems",
  showSidebar: true,
  showBottom: true,
  setSidebarWidth: (w) => set({ sidebarWidth: w }),
  setBottomHeight: (h) => set({ bottomHeight: h }),
  setSidebarTab: (t) => set({ sidebarTab: t }),
  setBottomTab: (t) => set({ bottomTab: t }),
  toggleSidebar: () => set((s) => ({ showSidebar: !s.showSidebar })),
  toggleBottom: () => set((s) => ({ showBottom: !s.showBottom })),
}));
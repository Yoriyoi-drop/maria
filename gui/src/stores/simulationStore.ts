import { create } from "zustand";

export interface SimSignal {
  name: string;
  width: number;
  value: string;
  timeline?: { time: number; value: string }[];
}

interface SimulationState {
  isRunning: boolean;
  maxTime: number;
  currentTime: number;
  signals: SimSignal[];
  compileResult: { success: boolean; errors: string[]; parseTime: number; elabTime: number } | null;
  setRunning: (v: boolean) => void;
  setMaxTime: (t: number) => void;
  setCurrentTime: (t: number) => void;
  setSignals: (s: SimSignal[]) => void;
  setCompileResult: (r: SimulationState["compileResult"]) => void;
}

export default create<SimulationState>((set) => ({
  isRunning: false,
  maxTime: 1000,
  currentTime: 0,
  signals: [],
  compileResult: null,
  setRunning: (v) => set({ isRunning: v }),
  setMaxTime: (t) => set({ maxTime: t }),
  setCurrentTime: (t) => set({ currentTime: t }),
  setSignals: (s) => set({ signals: s }),
  setCompileResult: (r) => set({ compileResult: r }),
}));
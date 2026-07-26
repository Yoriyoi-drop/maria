import { create } from "zustand";

export interface SimSignal {
  name: string;
  width: number;
  value: string;
  timeline?: { time: number; value: string }[];
}

export interface BenchmarkRun {
  id: number;
  timestamp: number;
  parseTime: number;
  elabTime: number;
  simTime: number;
  memoryMB: number;
  cpuPercent: number;
  modulesCount: number;
  signalsCount: number;
  throughputFiles: number;
}

export interface ResourceMetrics {
  cpuPercent: number;
  memoryMB: number;
  threads: number;
  queueDepth: number;
  filesPerSec: number;
}

interface SimulationState {
  isRunning: boolean;
  maxTime: number;
  currentTime: number;
  signals: SimSignal[];
  compileResult: { success: boolean; errors: string[]; parseTime: number; elabTime: number } | null;
  // Benchmark history
  benchmarkHistory: BenchmarkRun[];
  // Real-time resource metrics
  resourceMetrics: ResourceMetrics;
  setRunning: (v: boolean) => void;
  setMaxTime: (t: number) => void;
  setCurrentTime: (t: number) => void;
  setSignals: (s: SimSignal[]) => void;
  setCompileResult: (r: SimulationState["compileResult"]) => void;
  addBenchmarkRun: (run: BenchmarkRun) => void;
  clearBenchmarkHistory: () => void;
  updateResourceMetrics: (m: Partial<ResourceMetrics>) => void;
}

export default create<SimulationState>((set) => ({
  isRunning: false,
  maxTime: 1000,
  currentTime: 0,
  signals: [],
  compileResult: null,
  benchmarkHistory: [],
  resourceMetrics: {
    cpuPercent: 0,
    memoryMB: 0,
    threads: 0,
    queueDepth: 0,
    filesPerSec: 0,
  },
  setRunning: (v) => set({ isRunning: v }),
  setMaxTime: (t) => set({ maxTime: t }),
  setCurrentTime: (t) => set({ currentTime: t }),
  setSignals: (s) => set({ signals: s }),
  setCompileResult: (r) => set({ compileResult: r }),
  addBenchmarkRun: (run) =>
    set((s) => ({
      benchmarkHistory: [...s.benchmarkHistory, run],
    })),
  clearBenchmarkHistory: () => set({ benchmarkHistory: [] }),
  updateResourceMetrics: (m) =>
    set((s) => ({
      resourceMetrics: { ...s.resourceMetrics, ...m },
    })),
}));
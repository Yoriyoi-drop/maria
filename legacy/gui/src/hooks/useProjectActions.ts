import { useCallback } from "react";
import useProjectStore from "../stores/projectStore";
import useSimulationStore from "../stores/simulationStore";
import {
  openProject as ipcOpenProject,
  getFileTree as ipcGetFileTree,
  compileProject as ipcCompileProject,
  elaborateDesign as ipcElaborateDesign,
  runSimulation as ipcRunSimulation,
  getModules as ipcGetModules,
  getHierarchy as ipcGetHierarchy,
  getDependencies as ipcGetDependencies,
  searchSymbols as ipcSearchSymbols,
  grepSearch as ipcGrepSearch,
  getBenchmarkData as ipcGetBenchmarkData,
  getCoverageData as ipcGetCoverageData,
} from "./useMariaIPC";

export function useProjectActions() {
  const projectStore = useProjectStore();
  const simStore = useSimulationStore();

  // ── Open Project ──
  const openProject = useCallback(async (path: string) => {
    projectStore.setLoading(true);
    try {
      const project = await ipcOpenProject(path);
      projectStore.setProject(project.name, project.root);

      // Load file tree from backend
      const treeNodes = await ipcGetFileTree(project.root);
      projectStore.setFiles(
        treeNodes.map((n) => ({
          name: n.name,
          path: n.path,
          kind: n.kind as "file" | "directory",
          children: n.children?.map((c: any) => ({
            name: c.name,
            path: c.path,
            kind: c.kind as "file" | "directory",
            children: c.children,
          })),
        }))
      );

      // Load modules
      try {
        const modules = await ipcGetModules();
        projectStore.setModules(
          modules.map((m) => ({
            name: m.name,
            file: m.file,
            line: m.line,
            kind: m.kind as "module" | "interface" | "package" | "program" | "class",
          }))
        );
      } catch {
        // Modules not available until compilation
      }
    } catch (err: any) {
      projectStore.setDiagnostics([
        ...projectStore.diagnostics,
        { file: "", line: 0, message: `Failed to open project: ${err}`, level: "error" },
      ]);
    } finally {
      projectStore.setLoading(false);
    }
  }, [projectStore]);

  // ── Compile Project ──
  const compileProject = useCallback(async () => {
    const { rootPath, files } = projectStore;
    if (!rootPath) return;

    simStore.setRunning(false);

    // Collect all .sv file paths from file tree
    const collectPaths = (nodes: typeof files): string[] => {
      let paths: string[] = [];
      for (const n of nodes) {
        if (n.kind === "file" && (n.path.endsWith(".sv") || n.path.endsWith(".svh"))) {
          paths.push(n.path);
        }
        if (n.children) {
          paths = paths.concat(collectPaths(n.children));
        }
      }
      return paths;
    };

    const svPaths = collectPaths(files);

    if (svPaths.length === 0) {
      simStore.setCompileResult({
        success: false,
        errors: ["No .sv files found in project"],
        parseTime: 0,
        elabTime: 0,
      });
      projectStore.setDiagnostics([
        ...projectStore.diagnostics,
        { file: "", line: 0, message: "No .sv files found to compile", level: "error" },
      ]);
      return;
    }

    try {
      const result = await ipcCompileProject(svPaths);

      simStore.setCompileResult({
        success: result.success,
        errors: result.errors.map((e) => e.message),
        parseTime: result.parse_time_ms,
        elabTime: result.index_time_ms,
      });

      // Update project diagnostics
      const diags = [
        ...result.errors.map((e) => ({
          file: e.file,
          line: e.line,
          message: e.message,
          level: e.level as "error" | "warning" | "info",
        })),
        ...result.warnings.map((w) => ({
          file: w.file,
          line: w.line,
          message: w.message,
          level: "warning" as const,
        })),
      ];
      projectStore.setDiagnostics(diags);

      if (result.success) {
        // Update modules list
        const modules = result.modules.map((name) => ({
          name,
          file: "",
          line: 0,
          kind: "module" as const,
        }));
        projectStore.setModules(modules);

        // Try to elaborate (needed for simulation)
        try {
          await ipcElaborateDesign();
        } catch {
          // Elaboration will happen when running simulation
        }
      }
    } catch (err: any) {
      simStore.setCompileResult({
        success: false,
        errors: [String(err)],
        parseTime: 0,
        elabTime: 0,
      });
      projectStore.setDiagnostics([
        ...projectStore.diagnostics,
        { file: "", line: 0, message: `Compile error: ${err}`, level: "error" },
      ]);
    }
  }, [projectStore, simStore]);

  // ── Run / Stop Simulation ──
  const runSimulation = useCallback(async () => {
    const maxTime = simStore.maxTime;
    simStore.setRunning(true);

    try {
      // Elaborate if not already done
      try {
        await ipcElaborateDesign();
      } catch {
        // Already elaborated or will fail at run
      }

      const result = await ipcRunSimulation(maxTime);

      simStore.setSignals(
        result.signals.map((s) => ({
          name: s.name,
          width: s.width,
          value: s.value,
        }))
      );
      simStore.setCurrentTime(result.cycles);

      // Add benchmark run
      simStore.addBenchmarkRun({
        id: Date.now(),
        timestamp: Date.now(),
        parseTime: 0,
        elabTime: 0,
        simTime: result.sim_time_ms,
        memoryMB: 0,
        cpuPercent: 0,
        modulesCount: result.signals.length,
        signalsCount: result.signals.length,
        throughputFiles: 0,
      });
    } catch (err: any) {
      simStore.setCompileResult({
        success: false,
        errors: [String(err)],
        parseTime: 0,
        elabTime: 0,
      });
    } finally {
      simStore.setRunning(false);
    }
  }, [simStore]);

  const stopSimulation = useCallback(() => {
    simStore.setRunning(false);
  }, [simStore]);

  // ── Load Architecture ──
  const loadArchitecture = useCallback(async () => {
    try {
      const hierarchy = await ipcGetHierarchy();
      projectStore.setArchitecture(hierarchy);
    } catch {
      // Hierarchy not available
    }
  }, [projectStore]);

  // ── Load Dependencies ──
  const loadDependencies = useCallback(async () => {
    try {
      return await ipcGetDependencies();
    } catch {
      return [];
    }
  }, []);

  // ── Search ──
  const searchSymbols = useCallback(async (query: string) => {
    try {
      return await ipcSearchSymbols(query);
    } catch {
      return [];
    }
  }, []);

  const grepSearch = useCallback(async (pattern: string, path: string) => {
    try {
      return await ipcGrepSearch(pattern, path);
    } catch {
      return [];
    }
  }, []);

  // ── Benchmark ──
  const loadBenchmarkData = useCallback(async () => {
    try {
      return await ipcGetBenchmarkData();
    } catch {
      return null;
    }
  }, []);

  // ── Coverage ──
  const loadCoverageData = useCallback(async () => {
    try {
      return await ipcGetCoverageData();
    } catch {
      return null;
    }
  }, []);

  return {
    openProject,
    compileProject,
    runSimulation,
    stopSimulation,
    loadArchitecture,
    loadDependencies,
    searchSymbols,
    grepSearch,
    loadBenchmarkData,
    loadCoverageData,
  };
}

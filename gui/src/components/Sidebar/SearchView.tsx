import { useState, useRef, useEffect, useCallback } from "react";
import { Search, X, Code2, Cpu, Package as Pkg, Type, Variable, Hash, Filter } from "lucide-react";
import useEditorStore from "../../stores/editorStore";
import useProjectStore from "../../stores/projectStore";
import { grepSearch, searchSymbols, readFile } from "../../hooks/useMariaIPC";

type SearchFilter = "module" | "signal" | "parameter" | "package" | "macro" | "instance";

const filterColors: Record<SearchFilter, string> = {
  module: "var(--accent-blue)",
  signal: "var(--text-secondary)",
  parameter: "var(--accent-orange)",
  package: "var(--accent-cyan)",
  macro: "var(--text-muted)",
  instance: "var(--accent-purple)",
};

const filterIcons: Record<SearchFilter, any> = {
  module: Code2,
  signal: Cpu,
  parameter: Hash,
  package: Pkg,
  macro: Variable,
  instance: Type,
};

interface SearchResult {
  file: string;
  line: number;
  text: string;
  matchType: SearchFilter;
  matchText: string;
}

export default function SearchView() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [activeFilters, setActiveFilters] = useState<Set<SearchFilter>>(
    new Set(["module", "signal", "parameter", "package", "macro", "instance"])
  );
  const [showFilters, setShowFilters] = useState(false);
  const [isSearching, setIsSearching] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const { openFile, setFileContent } = useEditorStore();
  const { rootPath } = useProjectStore();

  // ── Search implementation ──
  const performSearch = useCallback(async (q: string) => {
    if (!q.trim()) {
      setResults([]);
      return;
    }

    setIsSearching(true);

    try {
      // Try backend grep search first
      if (rootPath) {
        const backendResults = await grepSearch(q, rootPath, "*.sv");
        if (backendResults.length > 0) {
          const mapped: SearchResult[] = backendResults.map((r) => {
            let matchType: SearchFilter = "signal";
            if (/\bmodule\s+\w+/.test(r.text)) matchType = "module";
            else if (/\bparameter\b/.test(r.text)) matchType = "parameter";
            else if (/\bpackage\b/.test(r.text)) matchType = "package";
            else if (/`\w+/.test(r.text)) matchType = "macro";
            else if (/\b\w+\s+u_\w+\s*\(/.test(r.text)) matchType = "instance";

            return {
              file: r.file,
              line: r.line,
              text: r.text,
              matchType,
              matchText: r.text,
            };
          });

          // Apply filters
          const filtered = mapped.filter((r) => activeFilters.has(r.matchType));

          // Deduplicate
          const seen = new Set<string>();
          const deduped = filtered.filter((r) => {
            const key = `${r.file}:${r.line}:${r.text}`;
            if (seen.has(key)) return false;
            seen.add(key);
            return true;
          });

          setResults(deduped.slice(0, 100));
          setIsSearching(false);
          return;
        }
      }

      // Fallback: try symbol search
      const symbols = await searchSymbols(q);
      if (symbols.length > 0) {
        const mapped: SearchResult[] = symbols.map((s) => ({
          file: s.file,
          line: s.line,
          text: s.text,
          matchType: s.match_type === "module" ? "module" : "signal",
          matchText: s.text,
        }));
        setResults(mapped.slice(0, 100));
        setIsSearching(false);
        return;
      }
    } catch {
      // Backend search failed — continue to mock search below
    }

    // Fallback: local mock search in file contents
    const mockFileContents: Record<string, string> = {
      "core/cpu_top.sv": `module cpu_top (\n  input  logic        clk,\n  input  logic        rst_n,\n  input  logic [31:0] instr,\n  output logic [31:0] result\n);`,
      "core/alu.sv": `module alu (\n  input  logic [31:0] a, b,\n  input  logic [3:0]  alu_op,\n  output logic [31:0] result,\n);`,
      "core/decoder.sv": `module decoder (\n  input  logic [31:0] instr,\n  output logic [3:0]  alu_op,\n);`,
    };

    const lower = q.toLowerCase();
    const found: SearchResult[] = [];

    for (const [file, content] of Object.entries(mockFileContents)) {
      const lines = content.split("\n");
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        if (!line.toLowerCase().includes(lower)) continue;

        let matchType: SearchFilter = "signal";
        if (/\bmodule\s+\w+/.test(line)) matchType = "module";
        else if (/\bparameter\b/.test(line)) matchType = "parameter";
        else if (/\bpackage\b/.test(line)) matchType = "package";
        else if (/`\w+/.test(line)) matchType = "macro";
        else if (/\b\w+\s+u_\w+\s*\(/.test(line)) matchType = "instance";

        found.push({ file, line: i + 1, text: line.trim(), matchType, matchText: line.trim() });
      }
    }

    const filtered = found.filter((r) => activeFilters.has(r.matchType));
    const seen = new Set<string>();
    const deduped = filtered.filter((r) => {
      const key = `${r.file}:${r.line}:${r.text}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });

    setResults(deduped.slice(0, 100));
    setIsSearching(false);
  }, [activeFilters, rootPath]);

  // Debounced search
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => performSearch(query), 150);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query, performSearch]);

  const toggleFilter = (f: SearchFilter) => {
    setActiveFilters((prev) => {
      const next = new Set(prev);
      if (next.has(f)) next.delete(f);
      else next.add(f);
      return next;
    });
  };

  const activeFilterCount = activeFilters.size;

  const handleOpenResult = async (r: SearchResult) => {
    const name = r.file.split("/").pop() || r.file;
    openFile(r.file, name);
    try {
      const content = await readFile(r.file);
      setFileContent(r.file, content);
    } catch {
      // File read error — skip
    }
  };

  return (
    <div>
      {/* ── Search Bar ── */}
      <div className="sidebar-section">
        <div className="sidebar-section__title">Search</div>
        <div style={{ position: "relative" }}>
          <Search size={13} style={{ position: "absolute", left: 8, top: 7, color: "var(--text-muted)" }} />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Find module, signal, parameter..."
            style={{
              width: "100%",
              padding: "5px 8px 5px 28px",
              borderRadius: 4,
              fontSize: 12,
            }}
          />
          {query && (
            <button
              onClick={() => setQuery("")}
              style={{ position: "absolute", right: 6, top: 6, color: "var(--text-muted)" }}
            >
              <X size={13} />
            </button>
          )}
        </div>
        <div style={{ display: "flex", gap: 4, marginTop: 4, alignItems: "center" }}>
          <div style={{ fontSize: 11, color: "var(--text-muted)", flex: 1 }}>
            {isSearching ? (
              <span style={{ color: "var(--accent-blue)" }}>Searching...</span>
            ) : results.length > 0 ? (
              `${results.length} result${results.length !== 1 ? "s" : ""}`
            ) : query ? "No results" : "Type to search"}
          </div>
          <button
            onClick={() => setShowFilters(!showFilters)}
            className="sidebar-item"
            style={{
              padding: "2px 6px",
              fontSize: 10,
              color: showFilters ? "var(--accent-blue)" : "var(--text-tertiary)",
              gap: 3,
            }}
          >
            <Filter size={11} />
            {activeFilterCount < 6 ? `${activeFilterCount} filters` : "All"}
          </button>
        </div>
      </div>

      {/* ── Filters ── */}
      {showFilters && (
        <div className="sidebar-section fade-in">
          <div className="sidebar-section__title" style={{ fontSize: 9 }}>
            Search Filters
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
            {(["module", "signal", "parameter", "package", "macro", "instance"] as SearchFilter[]).map((f) => (
              <button
                key={f}
                onClick={() => toggleFilter(f)}
                className="sidebar-item"
                style={{
                  padding: "2px 8px",
                  fontSize: 10,
                  borderRadius: 4,
                  gap: 4,
                  background: activeFilters.has(f) ? `${filterColors[f]}18` : "transparent",
                  border: activeFilters.has(f) ? `1px solid ${filterColors[f]}44` : "1px solid transparent",
                  color: activeFilters.has(f) ? filterColors[f] : "var(--text-tertiary)",
                }}
              >
                {f.charAt(0).toUpperCase() + f.slice(1)}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* ── Results ── */}
      {results.length > 0 && (
        <div>
          <div className="sidebar-section">
            <div className="sidebar-section__title">
              Search Results
              <span className="sidebar-item__badge" style={{ marginLeft: 6 }}>{results.length}</span>
            </div>
          </div>
          <div>
            {results.map((r, i) => {
              const Icon = filterIcons[r.matchType];
              const color = filterColors[r.matchType];

              return (
                <div
                  key={i}
                  className="sidebar-item"
                  style={{ alignItems: "flex-start", padding: "4px 12px 4px 12px" }}
                  onClick={() => handleOpenResult(r)}
                >
                  <Icon size={11} style={{ color, flexShrink: 0, marginTop: 2 }} />
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
                      <span className="sidebar-item__name" style={{ fontSize: 11 }}>
                        {r.file}
                      </span>
                      <span style={{ fontSize: 9, color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>
                        :{r.line}
                      </span>
                    </div>
                    <div
                      style={{
                        fontSize: 10,
                        color: "var(--text-tertiary)",
                        fontFamily: "var(--font-mono)",
                        whiteSpace: "nowrap",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        marginTop: 1,
                      }}
                    >
                      {r.text.length > 60 ? r.text.slice(0, 60) + "..." : r.text}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* ── Empty State ── */}
      {query && results.length === 0 && !isSearching && (
        <div
          style={{
            padding: "24px 16px",
            textAlign: "center",
            color: "var(--text-muted)",
            fontSize: 12,
          }}
        >
          <Search size={24} style={{ opacity: 0.3, marginBottom: 8 }} />
          <div>No results for <strong style={{ color: "var(--text-tertiary)" }}>"{query}"</strong></div>
          <div style={{ fontSize: 10, marginTop: 4 }}>Try different keywords or adjust filters</div>
        </div>
      )}

      {/* ── Initial state ── */}
      {!query && (
        <div style={{ padding: "16px", color: "var(--text-muted)", fontSize: 11, textAlign: "center" }}>
          Search through project files<br />
          <span style={{ fontSize: 10 }}>Try: "clk", "module", "always", "result"</span>
        </div>
      )}
    </div>
  );
}

import { useEffect, useRef, useMemo } from "react";
import * as monaco from "monaco-editor";
import useEditorStore from "../../stores/editorStore";
import { registerSVProviders } from "./svLanguageService";

const SV_LANGUAGE_ID = "systemverilog";
const SV_THEME_NAME = "maria-dark";

function registerSystemVerilog() {
  if (monaco.languages.getLanguages().some((l) => l.id === SV_LANGUAGE_ID)) return;

  monaco.languages.register({ id: SV_LANGUAGE_ID });

  // ── Semantic highlight token classes ──
  //   keyword.module   → module/endmodule (blue)
  //   keyword.interface→ interface/endinterface (purple)
  //   keyword.package  → package/endpackage (cyan)
  //   keyword.param    → parameter/localparam (orange)
  //   keyword.typedef  → typedef (green)
  //   keyword.enum     → enum (teal)
  //   keyword.clock    → clk/clock (yellow)
  //   keyword.reset    → rst/reset (red)
  //   macro            → `define, `ifdef etc (gray)
  //   type             → logic/reg/wire/bit etc (blue)
  //   keyword          → all other SV keywords (purple)
  //   identifier       → signal/variable names (white)

  const svKeywords = [
    "program", "endprogram", "class", "endclass", "function", "endfunction",
    "task", "endtask", "property", "endproperty", "sequence", "endsequence",
    "clocking", "endclocking", "checker", "endchecker", "primitive", "endprimitive",
    "config", "endconfig", "generate", "endgenerate", "specify", "endspecify",
    "input", "output", "inout", "ref",
    "string", "event", "struct", "union", "genvar", "automatic", "static", "virtual", "pure",
    "always", "always_comb", "always_ff", "always_latch", "initial", "final",
    "assign", "deassign", "force", "release", "if", "else", "case", "casex",
    "casez", "endcase", "for", "while", "repeat", "forever", "do",
    "begin", "end", "fork", "join", "join_any", "join_none",
    "disable", "wait", "assert", "assume", "cover", "rand", "randc",
    "constraint", "solve", "before", "dist", "unique", "priority",
    "new", "this", "super", "extends", "implements", "import", "export",
    "bind", "modport", "default", "global", "defparam",
    "signed", "unsigned", "pulsestyle_onevent", "pulsestyle_ondetect",
    "bufif0", "bufif1", "notif0", "notif1",
    "buf", "not", "and", "nand", "or", "nor", "xor", "xnor",
    "pullup", "pulldown", "strong0", "strong1", "weak0", "weak1",
    "highz0", "highz1", "tri", "tri0", "tri1", "triand", "trior", "trireg", "wand", "wor",
  ];

  const svTypeKeywords = [
    "bit", "logic", "reg", "wire", "byte", "int", "integer", "longint",
    "shortint", "time", "real", "realtime", "void",
  ];

  monaco.languages.setMonarchTokensProvider(SV_LANGUAGE_ID, {
    keywords: svKeywords,
    typeKeywords: svTypeKeywords,
    operators: [
      "=", ">", "<", "!", "~", "?", ":", "==", "<=", ">=", "!=",
      "&&", "||", "++", "--", "+", "-", "*", "/", "&", "|", "^", "%",
      "<<", ">>", "<<<", ">>>", "===", "!==", "*>", "->", "-:",
    ],
    symbols: /[=><!~?:&|+\-*/^%]+/,
    escapes: /\\(?:[abfnrtv\\"']|x[0-9A-Fa-f]{1,4}|u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8})/,

    tokenizer: {
      root: [
        // Macros first (highest priority)
        [/`\w+/, { token: "macro", next: "@macro" }],

        // Comments
        [/\/\/.*$/, "comment"],
        [/\/\*/, "comment", "@comment"],

        // Brackets and delimiters
        [/[{}()\[\]]/, "@brackets"],
        [/[;:,. ]/, "delimiter"],

        // Numbers — SV literals like 8'hFF, 32'd100
        [/\d+'[sS]?[bBoOdDhH]\s*[0-9a-fA-FzZxX?_]+/, "number"],
        [/\d*\.\d+([eE][-+]?\d+)?/, "number.float"],
        [/\d+/, "number"],

        // Strings
        [/"/, "string", "@string"],

        // ── Semantic Highlight: specific keywords with dedicated colors ──
        [/\b(module|endmodule)\b/, { token: "keyword.module" }],
        [/\b(interface|endinterface)\b/, { token: "keyword.interface" }],
        [/\b(package|endpackage)\b/, { token: "keyword.package" }],
        [/\b(parameter|localparam)\b/, { token: "keyword.param" }],
        [/\b(typedef)\b/, { token: "keyword.typedef" }],
        [/\b(enum)\b/, { token: "keyword.enum" }],
        [/\b(clk|clock)\b/i, { token: "keyword.clock" }],
        [/\b(rst|reset|rst_n|reset_n)\b/i, { token: "keyword.reset" }],

        // Identifiers: dispatch by type/keyword/default
        [/[a-zA-Z_]\w*/, {
          cases: {
            "@typeKeywords": "type",
            "@keywords": "keyword",
            "@default": "identifier",
          },
        }],
      ],

      macro: [
        [/`endif|`endcelldefine/, { token: "macro", next: "@pop" }],
        [/[^`\n]+/, "macro"],
        [/`\w+/, "macro"],
        [/\n/, { token: "macro", next: "@pop" }],
      ],

      comment: [
        [/[^\/*]+/, "comment"],
        [/\*\//, "comment", "@pop"],
        [/[\/*]/, "comment"],
      ],

      string: [
        [/[^\\"]+/, "string"],
        [/@escapes/, "string.escape"],
        [/\\./, "string.escape.invalid"],
        [/"/, "string", "@pop"],
      ],
    },
  } as monaco.languages.IMonarchLanguage);

  monaco.editor.defineTheme(SV_THEME_NAME, {
    base: "vs-dark",
    inherit: true,
    rules: [
      // General keyword (purple)
      { token: "keyword", foreground: "c586c0", fontStyle: "bold" },

      // Module/interface/package (structural)
      { token: "keyword.module", foreground: "4fc1ff", fontStyle: "bold" },
      { token: "keyword.interface", foreground: "a855f7", fontStyle: "bold" },
      { token: "keyword.package", foreground: "06b6d4", fontStyle: "bold" },

      // Parameters & types (orange/green/teal)
      { token: "keyword.param", foreground: "f97316" },
      { token: "keyword.typedef", foreground: "22c55e" },
      { token: "keyword.enum", foreground: "14b8a6" },

      // Clock (yellow) and Reset (red)
      { token: "keyword.clock", foreground: "eab308" },
      { token: "keyword.reset", foreground: "ef4444" },

      // Data types
      { token: "type", foreground: "4fc1ff" },

      // Macros, numbers, strings
      { token: "macro", foreground: "808080" },
      { token: "number", foreground: "b5cea8" },
      { token: "number.float", foreground: "b5cea8" },
      { token: "string", foreground: "ce9178" },
      { token: "string.escape", foreground: "d7ba7d" },

      // Comments
      { token: "comment", foreground: "6a9955", fontStyle: "italic" },

      // Default
      { token: "identifier", foreground: "d4d4d4" },
      { token: "delimiter", foreground: "71717a" },
    ],
    colors: {
      "editor.background": "#1a1b1e",
      "editor.foreground": "#d4d4d4",
      "editor.lineHighlightBackground": "#2a2d3a",
      "editorCursor.foreground": "#3b82f6",
      "editor.selectionBackground": "#3b82f644",
      "editor.inactiveSelectionBackground": "#3b82f622",
      "editorLineNumber.foreground": "#52525b",
      "editorLineNumber.activeForeground": "#a1a1aa",
      "editorGutter.background": "#1a1b1e",
      "editorBracketMatch.background": "#3b82f622",
      "editorBracketMatch.border": "#3b82f6",
      "editorWidget.background": "#222327",
      "editorWidget.border": "#2e2f34",
      "editorSuggestWidget.background": "#222327",
      "editorSuggestWidget.border": "#2e2f34",
      "editorSuggestWidget.selectedBackground": "#313236",
      "editorSuggestWidget.foreground": "#e4e4e7",
      "editorSuggestWidget.highlightForeground": "#3b82f6",
      "editorHint.foreground": "#22c55e",
      "editorInfo.foreground": "#3b82f6",
      "editorWarning.foreground": "#eab308",
      "editorError.foreground": "#ef4444",
      "minimap.background": "#1a1b1e",
      "minimap.errorHighlight": "#ef4444",
      "editorOverviewRuler.errorForeground": "#ef4444",
      "editorOverviewRuler.warningForeground": "#eab308",
      "editorOverviewRuler.infoForeground": "#3b82f6",
      "editorRuler.foreground": "#2e2f34",
      "editorStickyScroll.background": "#1e1f22",
      "editorStickyScroll.border": "#2e2f34",
    },
  });
}

export default function MonacoWrapper() {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const { activeFile, openFiles } = useEditorStore();
  const editorStore = useEditorStore();

  const activeContent = useMemo(() => {
    const f = openFiles.find((f) => f.path === activeFile);
    return f?.content || "";
  }, [openFiles, activeFile]);

  useEffect(() => {
    registerSystemVerilog();
    registerSVProviders();
  }, []);

  // Create editor
  useEffect(() => {
    if (!containerRef.current) return;

    const editor = monaco.editor.create(containerRef.current, {
      value: activeContent || "// Select a file to edit",
      language: SV_LANGUAGE_ID,
      theme: SV_THEME_NAME,
      automaticLayout: true,
      fontSize: 13,
      fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
      fontLigatures: true,
      lineNumbers: "on",
      minimap: { enabled: true, scale: 1, showSlider: "mouseover", renderCharacters: false },
      scrollBeyondLastLine: false,
      renderLineHighlight: "all",
      cursorBlinking: "smooth",
      cursorSmoothCaretAnimation: "on",
      smoothScrolling: true,
      bracketPairColorization: { enabled: true, independentColorPoolPerBracketType: true },
      padding: { top: 8 },
      folding: true,
      foldingHighlight: true,
      foldingStrategy: "indentation",
      guides: { indentation: true, bracketPairs: true, highlightActiveIndentation: true },
      wordWrap: "off",
      tabSize: 2,
      renderWhitespace: "selection",
      suggest: { showKeywords: true, showSnippets: true, showMethods: true, showFields: true },
      hover: { enabled: true, delay: 200, above: true },
      lightbulb: { enabled: "on" as any },
      codeLens: true,
      inlayHints: { enabled: "on" },
      stickyScroll: { enabled: true, maxLineCount: 5 },
      overviewRulerBorder: false,
      hideCursorInOverviewRuler: false,
      multiCursorModifier: "alt",
      accessibilitySupport: "off",
    });

    editorRef.current = editor;

    editor.onDidChangeModelContent(() => {
      const val = editor.getValue();
      if (activeFile) {
        editorStore.setFileContent(activeFile, val);
        editorStore.markDirty(activeFile);
      }
    });

    return () => editor.dispose();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [containerRef]);

  // Sync active file content
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    if (!activeFile) return;

    const f = openFiles.find((f) => f.path === activeFile);
    if (f?.content !== undefined && f.content !== editor.getValue()) {
      editor.setValue(f.content);
    }
  }, [activeFile, openFiles]);

  return (
    <>
      <div className="editor-breadcrumb">
        {activeFile?.split("/").map((part, i, arr) => (
          <span key={i} style={{ display: "flex", alignItems: "center", gap: 4 }}>
            {i > 0 && <span className="editor-breadcrumb__sep">›</span>}
            <span
              className={`editor-breadcrumb__item ${i === arr.length - 1 ? "editor-breadcrumb__item--current" : ""}`}
            >
              {part}
            </span>
          </span>
        ))}
      </div>
      <div ref={containerRef} style={{ width: "100%", height: "calc(100% - 24px)" }} />
    </>
  );
}

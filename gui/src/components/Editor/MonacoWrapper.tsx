import { useEffect, useRef, useMemo } from "react";
import * as monaco from "monaco-editor";
import useEditorStore from "../../stores/editorStore";

const SV_LANGUAGE_ID = "systemverilog";
const SV_THEME_NAME = "maria-dark";

function registerSystemVerilog() {
  monaco.languages.register({ id: SV_LANGUAGE_ID });

  monaco.languages.setMonarchTokensProvider(SV_LANGUAGE_ID, {
    keywords: [
      "module", "endmodule", "interface", "endinterface", "package", "endpackage",
      "program", "endprogram", "class", "endclass", "function", "endfunction",
      "task", "endtask", "property", "endproperty", "sequence", "endsequence",
      "clocking", "endclocking", "checker", "endchecker", "primitive", "endprimitive",
      "config", "endconfig", "generate", "endgenerate", "specify", "endspecify",
      "input", "output", "inout", "ref", "wire", "reg", "logic", "bit", "byte",
      "int", "integer", "longint", "shortint", "time", "real", "realtime",
      "string", "event", "struct", "union", "enum", "typedef", "parameter",
      "localparam", "genvar", "automatic", "static", "virtual", "pure",
      "always", "always_comb", "always_ff", "always_latch", "initial", "final",
      "assign", "deassign", "force", "release", "if", "else", "case", "casex",
      "casez", "endcase", "for", "while", "repeat", "forever", "do",
      "begin", "end", "fork", "join", "join_any", "join_none",
      "disable", "wait", "assert", "assume", "cover", "rand", "randc",
      "constraint", "solve", "before", "dist", "unique", "priority",
      "new", "this", "super", "extends", "implements", "import", "export",
      "bind", "modport", "clocking", "default", "global", "defparam",
      "signed", "unsigned", "pulsestyle_onevent", "pulsestyle_ondetect",
      "cmos", "nmos", "pmos", "tran", "tranif0", "tranif1", "rtran",
      "rtranif0", "rtranif1", "bufif0", "bufif1", "notif0", "notif1",
      "buf", "not", "and", "nand", "or", "nor", "xor", "xnor",
      "pullup", "pulldown", "strong0", "strong1", "weak0", "weak1",
      "highz0", "highz1", "small", "medium", "large", "supply0", "supply1",
      "tri", "tri0", "tri1", "triand", "trior", "trireg", "wand", "wor",
    ],
    typeKeywords: [
      "bit", "logic", "reg", "wire", "byte", "int", "integer", "longint",
      "shortint", "time", "real", "realtime", "string", "event", "void",
    ],
    operators: [
      "=", ">", "<", "!", "~", "?", ":", "==", "<=", ">=", "!=",
      "&&", "||", "++", "--", "+", "-", "*", "/", "&", "|", "^", "%",
      "<<", ">>", "<<<", ">>>", "===", "!==", "*>", "->", "-:",
    ],
    symbols: /[=><!~?:&|+\-*/^%]+/,
    escapes: /\\(?:[abfnrtv\\"']|x[0-9A-Fa-f]{1,4}|u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8})/,
    tokenizer: {
      root: [
        [/\`\w+/, "macro"],
        [/\/\/.*$/, "comment"],
        [/\/\*/, "comment", "@comment"],
        [/[{}()\[\]]/, "@brackets"],
        [/[;:,. ]/, "delimiter"],
        [/\d+'\s*[bBoOdDhH]\s*[0-9a-fzZxX?_]+/, "number"],
        [/\d+'\s*[bBoOdDhH]\s*[0-9a-fzZxX?_]+/, "number"],
        [/\d*\.\d+([eE][-+]?\d+)?/, "number.float"],
        [/\d+/, "number"],
        [/"([^"\\]|\\.)*$/, "string.invalid"],
        [/"/, "string", "@string"],
        [/[a-zA-Z_]\w*/, { cases: { "@typeKeywords": "type",
                                       "@keywords": "keyword",
                                       "@default": "identifier" } }],
        [/[a-zA-Z_]\w*/, "identifier"],
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
  } as any);

  monaco.editor.defineTheme(SV_THEME_NAME, {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "keyword", foreground: "c586c0", fontStyle: "bold" },
      { token: "type", foreground: "4fc1ff" },
      { token: "macro", foreground: "808080" },
      { token: "number", foreground: "b5cea8" },
      { token: "string", foreground: "ce9178" },
      { token: "comment", foreground: "6a9955", fontStyle: "italic" },
      { token: "identifier", foreground: "d4d4d4" },
    ],
    colors: {
      "editor.background": "#1a1b1e",
      "editor.foreground": "#d4d4d4",
      "editor.lineHighlightBackground": "#2a2d3a",
      "editorCursor.foreground": "#3b82f6",
      "editor.selectionBackground": "#3b82f644",
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
      "editorHint.foreground": "#22c55e",
      "editorInfo.foreground": "#3b82f6",
      "editorWarning.foreground": "#eab308",
      "editorError.foreground": "#ef4444",
      "minimap.background": "#1a1b1e",
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
    return f?.content || "// No content";
  }, [openFiles, activeFile]);

  useEffect(() => {
    registerSystemVerilog();
  }, []);

  useEffect(() => {
    if (!containerRef.current) return;

    const editor = monaco.editor.create(containerRef.current, {
      value: activeContent,
      language: SV_LANGUAGE_ID,
      theme: SV_THEME_NAME,
      automaticLayout: true,
      fontSize: 13,
      fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
      fontLigatures: true,
      lineNumbers: "on",
      minimap: { enabled: true, scale: 1, showSlider: "mouseover" },
      scrollBeyondLastLine: false,
      renderLineHighlight: "line",
      cursorBlinking: "smooth",
      cursorSmoothCaretAnimation: "on",
      smoothScrolling: true,
      bracketPairColorization: { enabled: true },
      padding: { top: 8 },
      folding: true,
      foldingHighlight: true,
      guides: { indentation: true, bracketPairs: true },
      wordWrap: "off",
      tabSize: 2,
      renderWhitespace: "selection",
      suggest: { showKeywords: true, showSnippets: true },
      hover: { enabled: true, delay: 300 },
      lightbulb: { enabled: true },
      codeLens: true,
      inlayHints: { enabled: "on" },
      stickyScroll: { enabled: true },
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
  }, [containerRef.current]);

  useEffect(() => {
    const editor = editorRef.current;
    if (editor && activeFile) {
      const f = openFiles.find((f) => f.path === activeFile);
      if (f?.content !== undefined) {
        editor.setValue(f.content);
      }
    }
  }, [activeFile]);

  return (
    <>
      <div className="editor-breadcrumb">
        {activeFile?.split("/").map((part, i, arr) => (
          <span key={i} style={{ display: "flex", alignItems: "center", gap: 4 }}>
            {i > 0 && <span className="editor-breadcrumb__sep">›</span>}
            <span className={`editor-breadcrumb__item ${i === arr.length - 1 ? "editor-breadcrumb__item--current" : ""}`}>
              {part}
            </span>
          </span>
        ))}
      </div>
      <div ref={containerRef} style={{ width: "100%", height: "calc(100% - 24px)" }} />
    </>
  );
}
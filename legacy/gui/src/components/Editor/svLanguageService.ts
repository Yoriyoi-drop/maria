import * as monaco from "monaco-editor";

const SV_LANGUAGE_ID = "systemverilog";

// ── Mock project metadata (simulated backend data) ──
const moduleMeta: Record<string, {
  implementations: number;
  references: number;
  compileTime: number;
  coverage: number;
  signals: { name: string; type: string; width: number; line: number; usageCount: number; lastAssign: number }[];
}> = {
  "cpu_top": {
    implementations: 47, references: 312, compileTime: 0.31, coverage: 98,
    signals: [
      { name: "clk", type: "logic", width: 1, line: 3, usageCount: 12, lastAssign: 0 },
      { name: "rst_n", type: "logic", width: 1, line: 4, usageCount: 8, lastAssign: 0 },
      { name: "instr", type: "logic", width: 32, line: 5, usageCount: 3, lastAssign: 0 },
      { name: "result", type: "logic", width: 32, line: 6, usageCount: 5, lastAssign: 18 },
      { name: "pc", type: "logic", width: 32, line: 8, usageCount: 4, lastAssign: 16 },
      { name: "valid", type: "logic", width: 1, line: 9, usageCount: 2, lastAssign: 0 },
      { name: "ready", type: "logic", width: 1, line: 10, usageCount: 1, lastAssign: 0 },
    ],
  },
  "alu": {
    implementations: 23, references: 156, compileTime: 0.12, coverage: 95,
    signals: [
      { name: "a", type: "logic", width: 32, line: 2, usageCount: 6, lastAssign: 0 },
      { name: "b", type: "logic", width: 32, line: 2, usageCount: 6, lastAssign: 0 },
      { name: "alu_op", type: "logic", width: 4, line: 3, usageCount: 4, lastAssign: 0 },
      { name: "result", type: "logic", width: 32, line: 4, usageCount: 7, lastAssign: 14 },
      { name: "zero", type: "logic", width: 1, line: 5, usageCount: 2, lastAssign: 15 },
    ],
  },
  "decoder": {
    implementations: 15, references: 98, compileTime: 0.08, coverage: 91,
    signals: [
      { name: "instr", type: "logic", width: 32, line: 2, usageCount: 4, lastAssign: 0 },
      { name: "alu_op", type: "logic", width: 4, line: 3, usageCount: 2, lastAssign: 11 },
      { name: "reg_write", type: "logic", width: 1, line: 4, usageCount: 2, lastAssign: 12 },
      { name: "mem_read", type: "logic", width: 1, line: 5, usageCount: 2, lastAssign: 13 },
      { name: "opcode", type: "logic", width: 7, line: 7, usageCount: 2, lastAssign: 7 },
    ],
  },
  "cache_controller": {
    implementations: 8, references: 67, compileTime: 0.21, coverage: 97,
    signals: [
      { name: "clk", type: "logic", width: 1, line: 2, usageCount: 9, lastAssign: 0 },
      { name: "rst_n", type: "logic", width: 1, line: 3, usageCount: 6, lastAssign: 0 },
      { name: "addr", type: "logic", width: 32, line: 4, usageCount: 4, lastAssign: 0 },
      { name: "rd_en", type: "logic", width: 1, line: 5, usageCount: 3, lastAssign: 0 },
      { name: "rd_data", type: "logic", width: 32, line: 6, usageCount: 2, lastAssign: 0 },
      { name: "hit", type: "logic", width: 1, line: 7, usageCount: 5, lastAssign: 31 },
      { name: "state", type: "state_t", width: 2, line: 19, usageCount: 7, lastAssign: 27 },
      { name: "next", type: "state_t", width: 2, line: 19, usageCount: 7, lastAssign: 28 },
      { name: "cache_data", type: "logic", width: 32, line: 20, usageCount: 2, lastAssign: 0 },
    ],
  },
};

// ── Known module, package, interface names for autocomplete ──
const knownModules = [
  "cpu_top", "alu", "decoder", "cache_controller",
  "axi_crossbar", "axi2apb", "ddr_controller",
  "gpio", "uart", "spi_master", "i2c_controller", "timer",
  "sram_wrap", "rom",
];
const knownPackages = ["uvm_pkg", "sv_pkg", "axi_pkg", "common_pkg"];
const knownInterfaces = ["axi_if", "apb_if", "wishbone_if", "bus_if"];

// ── Parse helpers ──

/** Extract module name from a line like "module cpu_top (...)" */
function extractModuleName(line: string): string | null {
  const m = line.match(/\bmodule\s+(\w+)/);
  return m ? m[1] : null;
}

/** Find the word at position in text */
function getWordAtPosition(model: monaco.editor.ITextModel, pos: monaco.Position): string | null {
  const word = model.getWordAtPosition(pos);
  return word ? word.word : null;
}

// ── 1. CodeLensProvider ──

class SVCodeLensProvider implements monaco.languages.CodeLensProvider {
  async provideCodeLenses(model: monaco.editor.ITextModel): Promise<monaco.languages.CodeLensList> {
    const lenses: monaco.languages.CodeLens[] = [];
    const lines = model.getValue().split("\n");

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const modName = extractModuleName(line);
      if (!modName) continue;

      const range = new monaco.Range(i + 1, 1, i + 1, 1);
      const meta = moduleMeta[modName];

      if (meta) {
        lenses.push({
          range,
          id: `${modName}-impl`,
          command: {
            id: "_sv_codelens_noop",
            title: `⇅ Implemented by ${meta.implementations} files  ·  Referenced ${meta.references} times`,
            tooltip: `Module ${modName} is instantiated in ${meta.implementations} files and referenced ${meta.references} times`,
          },
        });
        lenses.push({
          range,
          id: `${modName}-cov`,
          command: {
            id: "_sv_codelens_noop",
            title: `◉ Compile ${meta.compileTime.toFixed(2)}ms  ·  Coverage ${meta.coverage}%`,
            tooltip: `Compile time: ${meta.compileTime.toFixed(2)}ms  ·  Coverage: ${meta.coverage}%`,
          },
        });
      } else {
        lenses.push({
          range,
          id: `${modName}-unknown`,
          command: {
            id: "_sv_codelens_noop",
            title: `◉ No metadata available`,
            tooltip: `No compilation data for module "${modName}"`,
          },
        });
      }
    }

    return { lenses, dispose: () => {} };
  }

  resolveCodeLens?(_model: monaco.editor.ITextModel, codeLens: monaco.languages.CodeLens) {
    return codeLens;
  }
}

// ── 2. CompletionItemProvider ──

class SVCompletionProvider implements monaco.languages.CompletionItemProvider {
  triggerCharacters = [".", ":", ":", "`", "_"];

  async provideCompletionItems(
    model: monaco.editor.ITextModel,
    position: monaco.Position
  ): Promise<monaco.languages.CompletionList> {
    const wordUntil = model.getWordUntilPosition(position);
    const word = wordUntil.word;
    const lineContent = model.getLineContent(position.lineNumber);
    const lineBefore = lineContent.substring(0, position.column - 1).toLowerCase();

    const suggestions: monaco.languages.CompletionItem[] = [];
    const range = {
      startLineNumber: position.lineNumber,
      endLineNumber: position.lineNumber,
      startColumn: wordUntil.startColumn,
      endColumn: wordUntil.endColumn,
    };

    // ── After "import" keyword ──
    if (/\bimport\s+$/.test(lineBefore) || /\bimport\s+\w*$/.test(lineBefore)) {
      for (const pkg of knownPackages) {
        suggestions.push({
          label: pkg,
          kind: monaco.languages.CompletionItemKind.Module,
          detail: "package",
          insertText: `${pkg}::*`,
          range,
        });
      }
      for (const iface of knownInterfaces) {
        suggestions.push({
          label: iface,
          kind: monaco.languages.CompletionItemKind.Interface,
          detail: "interface",
          insertText: iface,
          range,
        });
      }
    }

    // ── After "module" keyword (module name) ──
    if (/\bmodule\s+$/.test(lineBefore) || /\bmodule\s+\w*$/.test(lineBefore)) {
      for (const mod of knownModules) {
        suggestions.push({
          label: mod,
          kind: monaco.languages.CompletionItemKind.Class,
          detail: "module",
          insertText: mod,
          range,
        });
      }
    }

    // ── After "." (member access) ──
    if (lineBefore.endsWith(".")) {
      const members = [
        { label: "clk", detail: "logic 1-bit — clock input", kind: monaco.languages.CompletionItemKind.Field },
        { label: "rst_n", detail: "logic 1-bit — reset input", kind: monaco.languages.CompletionItemKind.Field },
        { label: "addr", detail: "logic 32-bit — address bus", kind: monaco.languages.CompletionItemKind.Field },
        { label: "data", detail: "logic 32-bit — data bus", kind: monaco.languages.CompletionItemKind.Field },
        { label: "valid", detail: "logic 1-bit — valid signal", kind: monaco.languages.CompletionItemKind.Field },
        { label: "ready", detail: "logic 1-bit — ready signal", kind: monaco.languages.CompletionItemKind.Field },
      ];
      for (const m of members) {
        suggestions.push({ ...m, insertText: m.label, range });
      }
    }

    // ── Signal names from current document ──
    const lines = model.getValue().split("\n");
    const localSignals = new Set<string>();
    for (const l of lines) {
      // Match "logic [WIDTH] NAME" or "logic NAME" or "input/output logic NAME"
      const sigMatch = l.match(/(?:input\s+|output\s+|inout\s+)?(?:logic|reg|wire|bit)\s*(?:\[[^\]]*\])?\s+(\w+)/);
      if (sigMatch) localSignals.add(sigMatch[1]);
      // Match "TYPE NAME" declarations
      const declMatch = l.match(/^\s*(?:logic|reg|wire|bit|int|byte)\s+(?:\w+\s*,\s*)*(\w+)\s*[=;]/);
      if (declMatch) localSignals.add(declMatch[1]);
    }

    for (const sig of localSignals) {
      if (!word || sig.toLowerCase().startsWith(word.toLowerCase())) {
        suggestions.push({
          label: sig,
          kind: monaco.languages.CompletionItemKind.Variable,
          detail: "signal",
          insertText: sig,
          range,
        });
      }
    }

    // ── Contextual: inside always_ff detect clock/reset ──
    if (lineBefore.includes("posedge") || lineBefore.includes("negedge")) {
      suggestions.push({
        label: "clk",
        kind: monaco.languages.CompletionItemKind.Keyword,
        detail: "clock signal",
        insertText: "clk",
        range,
      });
      suggestions.push({
        label: "rst_n",
        kind: monaco.languages.CompletionItemKind.Keyword,
        detail: "reset signal",
        insertText: "rst_n",
        range,
      });
    }

    // ── Keywords (when typing starts) ──
    if (!word || word.length <= 3 || suggestions.length < 3) {
      const kws = [
        { label: "always_ff", detail: "procedural block — flip-flop" },
        { label: "always_comb", detail: "procedural block — combinational" },
        { label: "always_latch", detail: "procedural block — latch" },
        { label: "module", detail: "module declaration" },
        { label: "endmodule", detail: "end module declaration" },
        { label: "input", detail: "port direction — input" },
        { label: "output", detail: "port direction — output" },
        { label: "inout", detail: "port direction — bidirectional" },
        { label: "logic", detail: "data type — 4-state logic" },
        { label: "reg", detail: "data type — register" },
        { label: "wire", detail: "data type — wire" },
        { label: "bit", detail: "data type — 2-state bit" },
        { label: "int", detail: "data type — integer" },
        { label: "assign", detail: "continuous assignment" },
        { label: "case", detail: "case statement" },
        { label: "endcase", detail: "end case statement" },
        { label: "if", detail: "conditional branch" },
        { label: "else", detail: "conditional else branch" },
        { label: "for", detail: "loop construct" },
        { label: "begin", detail: "block begin" },
        { label: "end", detail: "block end" },
        { label: "fork", detail: "fork block" },
        { label: "join", detail: "join block" },
        { label: "typedef", detail: "type definition" },
        { label: "enum", detail: "enumeration type" },
        { label: "struct", detail: "structure type" },
        { label: "parameter", detail: "parameter declaration" },
        { label: "localparam", detail: "local parameter" },
        { label: "import", detail: "package import" },
        { label: "package", detail: "package declaration" },
        { label: "endpackage", detail: "end package" },
        { label: "interface", detail: "interface declaration" },
        { label: "endinterface", detail: "end interface" },
        { label: "class", detail: "class declaration" },
        { label: "endclass", detail: "end class" },
        { label: "function", detail: "function declaration" },
        { label: "endfunction", detail: "end function" },
        { label: "task", detail: "task declaration" },
        { label: "endtask", detail: "end task" },
        { label: "foreach", detail: "foreach loop" },
        { label: "generate", detail: "generate block" },
        { label: "endgenerate", detail: "end generate" },
        { label: "assert", detail: "assertion" },
        { label: "cover", detail: "cover point" },
        { label: "rand", detail: "randomize modifier" },
        { label: "constraint", detail: "constraint block" },
        { label: "new", detail: "constructor / new" },
        { label: "this", detail: "this reference" },
        { label: "super", detail: "superclass reference" },
        { label: "extends", detail: "class inheritance" },
        { label: "virtual", detail: "virtual modifier" },
        { label: "pure", detail: "pure virtual" },
        { label: "static", detail: "static modifier" },
        { label: "automatic", detail: "automatic storage" },
        { label: "modport", detail: "interface modport" },
        { label: "clocking", detail: "clocking block" },
        { label: "default", detail: "default case/assignment" },
        { label: "unique", detail: "unique case/if" },
        { label: "priority", detail: "priority case/if" },
        { label: "signed", detail: "signed modifier" },
        { label: "unsigned", detail: "unsigned modifier" },
        { label: "void", detail: "void return type" },
        { label: "return", detail: "return statement" },
        { label: "disable", detail: "disable block" },
        { label: "wait", detail: "wait statement" },
        { label: "forever", detail: "infinite loop" },
        { label: "repeat", detail: "repeat loop" },
        { label: "while", detail: "while loop" },
        { label: "do", detail: "do-while loop" },
      ];

      for (const kw of kws) {
        if (!word || kw.label.startsWith(word.toLowerCase())) {
          suggestions.push({
            label: kw.label,
            kind: monaco.languages.CompletionItemKind.Keyword,
            detail: kw.detail,
            insertText: kw.label,
            range,
          });
        }
      }
    }

    return { suggestions };
  }

  resolveCompletionItem?(item: monaco.languages.CompletionItem) {
    return item;
  }
}

// ── 3. HoverProvider ──

class SVHoverProvider implements monaco.languages.HoverProvider {
  async provideHover(
    model: monaco.editor.ITextModel,
    position: monaco.Position
  ): Promise<monaco.languages.Hover | undefined> {
    const word = getWordAtPosition(model, position);
    if (!word) return;

    const lineContent = model.getLineContent(position.lineNumber);

    // ── Check if this is a known signal in any module ──
    for (const [modName, meta] of Object.entries(moduleMeta)) {
      const sig = meta.signals.find((s) => s.name === word);
      if (!sig) continue;

      const totalUsage = sig.usageCount;
      const declLine = sig.line;

      const hoverContent = [
        `**\`${sig.name}\`** — ${sig.type}  \`[${sig.width}:0]\``,
        `---`,
        `| | |`,
        `|---|---|`,
        `| **Declared** | \`${modName}\` line ${declLine} |`,
        `| **Width** | ${sig.width} bit${sig.width !== 1 ? "s" : ""} |`,
        `| **Type** | \`${sig.type}\` |`,
        `| **Usage** | ${totalUsage} reference${totalUsage !== 1 ? "s" : ""} |`,
        `| **Coverage** | ${meta.coverage}% |`,
      ];

      if (sig.lastAssign > 0) {
        hoverContent.push(`| **Last Assignment** | line ${sig.lastAssign} |`);
      }

      return {
        contents: [{ value: hoverContent.join("\n") }],
      };
    }

    // ── Check if hovering over a module name ──
    const modMatch = lineContent.match(/\bmodule\s+(\w+)/);
    if (modMatch && modMatch[1] === word) {
      const meta = moduleMeta[word];
      if (meta) {
        return {
          contents: [{
            value: [
              `**Module \`${word}\`**`,
              `---`,
              `| | |`,
              `|---|---|`,
              `| **Implementations** | ${meta.implementations} files |`,
              `| **References** | ${meta.references} times |`,
              `| **Compile Time** | ${meta.compileTime.toFixed(2)} ms |`,
              `| **Coverage** | ${meta.coverage}% |`,
              `| **Signals** | ${meta.signals.length} total |`,
            ].join("\n"),
          }],
        };
      }
    }

    // ── Generic hover: show line context ──
    const normalized = lineContent.trim();
    if (normalized && normalized.length > 0) {
      // Detect declaration pattern
      const declMatch = normalized.match(
        /(input|output|inout|ref)?\s*(logic|reg|wire|bit|int|byte|integer|time|real)\s*(\[[^\]]*\])?\s*(\w+)/
      );
      if (declMatch) {
        const dir = declMatch[1] || "internal";
        const dtype = declMatch[2];
        const width = declMatch[3] || "[0:0]";
        const name = declMatch[4];
        if (name === word) {
          return {
            contents: [{
              value: `**\`${name}\`** — ${dtype} ${width}\n\n_Direction_: ${dir}\n_Declared at line ${position.lineNumber}_`,
            }],
          };
        }
      }
    }

    return;
  }
}

// ── 4. DefinitionProvider ──

class SVDefinitionProvider implements monaco.languages.DefinitionProvider {
  async provideDefinition(
    model: monaco.editor.ITextModel,
    position: monaco.Position
  ): Promise<monaco.languages.Definition | undefined> {
    const word = getWordAtPosition(model, position);
    if (!word) return;

    const lines = model.getValue().split("\n");

    // Search for the definition of this identifier
    // Patterns: "logic NAME", "input logic NAME", "output NAME", "wire NAME", "reg NAME", "TYPE NAME"
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();

      // Skip comments
      if (line.startsWith("//") || line.startsWith("/*")) continue;

      // Match signal/port declarations: "logic [WIDTH] NAME" or "input logic NAME" or "TYPE NAME"
      const declPattern = new RegExp(
        `(?:input|output|inout|ref)?\\s*(?:logic|reg|wire|bit|int|byte|integer|time|real)\\s*(?:\\[[^\\]]*\\])?\\s+\\b${word}\\b`
      );
      if (declPattern.test(line)) {
        return {
          uri: model.uri,
          range: new monaco.Range(i + 1, line.indexOf(word) + 1, i + 1, line.indexOf(word) + 1 + word.length),
        };
      }

      // Match variable assignment: "NAME <= ..." or "NAME = ..."
      const assignPattern = new RegExp(`^\\s*\\b${word}\\b\\s*(<=|=)`);
      if (assignPattern.test(line)) {
        return {
          uri: model.uri,
          range: new monaco.Range(i + 1, line.indexOf(word) + 1, i + 1, line.indexOf(word) + 1 + word.length),
        };
      }

      // Match module/class/function/task definitions
      const defPattern = new RegExp(`\\b(?:module|class|function|task|interface|package)\\s+\\b${word}\\b`);
      if (defPattern.test(line)) {
        const match = line.match(new RegExp(`\\b${word}\\b`));
        if (match) {
          return {
            uri: model.uri,
            range: new monaco.Range(i + 1, match.index! + 1, i + 1, match.index! + 1 + word.length),
          };
        }
      }
    }

    return;
  }
}

// ── Registration ──

let providersRegistered = false;

export function registerSVProviders() {
  if (providersRegistered) return;

  monaco.languages.registerCodeLensProvider(SV_LANGUAGE_ID, new SVCodeLensProvider());
  monaco.languages.registerCompletionItemProvider(SV_LANGUAGE_ID, new SVCompletionProvider());
  monaco.languages.registerHoverProvider(SV_LANGUAGE_ID, new SVHoverProvider());
  monaco.languages.registerDefinitionProvider(SV_LANGUAGE_ID, new SVDefinitionProvider());

  // Register the CodeLens command (required for lenses to display)
  monaco.editor.registerCommand("_sv_codelens_noop", () => {
    // no-op — lenses are display-only
  });

  providersRegistered = true;
}

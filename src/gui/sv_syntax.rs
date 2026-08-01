//! Definisi syntax SystemVerilog untuk egui_code_editor.
//!
//! Keyword/type/special diambil dari konvensi semantic highlight Monaco lama
//! (lihat legacy/gui) agar warna yang dihasilkan konsisten.

use std::collections::BTreeSet;

use egui_code_editor::Syntax;

/// Keyword struktural + kontrol SystemVerilog.
const SV_KEYWORDS: &[&str] = &[
    "module", "endmodule", "interface", "endinterface", "package", "endpackage",
    "program", "endprogram", "class", "endclass", "function", "endfunction",
    "task", "endtask", "property", "endproperty", "sequence", "endsequence",
    "clocking", "endclocking", "checker", "endchecker", "primitive", "endprimitive",
    "config", "endconfig", "generate", "endgenerate", "specify", "endspecify",
    "input", "output", "inout", "ref",
    "always", "always_comb", "always_ff", "always_latch", "initial", "final",
    "assign", "deassign", "force", "release",
    "if", "else", "case", "casex", "casez", "endcase",
    "for", "while", "repeat", "forever", "do",
    "begin", "end", "fork", "join", "join_any", "join_none",
    "disable", "wait", "assert", "assume", "cover",
    "rand", "randc", "constraint", "solve", "before", "dist",
    "unique", "priority", "new", "this", "super", "extends", "implements",
    "import", "export", "bind", "modport", "default", "global", "defparam",
    "signed", "unsigned", "genvar", "automatic", "static", "virtual", "pure",
    "typedef", "enum", "struct", "union", "return", "break", "continue",
    "void", "local", "extern", "protected", "var", "parameter", "localparam",
];

/// Tipe data SystemVerilog.
const SV_TYPES: &[&str] = &[
    "bit", "logic", "reg", "wire", "byte", "int", "integer", "longint",
    "shortint", "time", "real", "realtime", "string", "event",
];

/// Fungsi sistem ($display, $finish, dll).
const SV_SYSTEM_FUNCS: &[&str] = &[
    "$display", "$write", "$strobe", "$monitor", "$finish", "$stop",
    "$fatal", "$error", "$warning", "$info", "$time", "$realtime",
    "$clog2", "$bits", "$size", "$left", "$right", "$low", "$high",
    "$urandom", "$random", "$sformatf", "$fopen", "$fclose", "$fdisplay",
    "$fwrite", "$fstrobe", "$fmonitor", "$fscanf", "$fread", "$readmemh",
    "$readmemb", "$value$plusargs", "$signed", "$unsigned",
];

/// Konstruksi Syntax SystemVerilog untuk egui_code_editor.
pub fn systemverilog_syntax() -> Syntax {
    Syntax::new("systemverilog")
        .with_comment("//")
        .with_comment_multiline(["/*", "*/"])
        .with_quotes(['"'])
        .with_keywords(SV_KEYWORDS.iter().copied().collect::<BTreeSet<_>>())
        .with_types(SV_TYPES.iter().copied().collect::<BTreeSet<_>>())
        .with_special(SV_SYSTEM_FUNCS.iter().copied().collect::<BTreeSet<_>>())
        .with_word_start(['$'])
}

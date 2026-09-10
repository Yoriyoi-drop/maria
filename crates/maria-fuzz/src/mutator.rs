//! Mutasi source SystemVerilog (LangFuzz-style, REDQUEEN-inspired).
//!
//! Semua operasi bekerja pada seed nyata. Operasi byte-safe (char-boundary)
//! untuk hindari panic slicing unicode (temuan fuzz nyata di mvm-fuzz).

use crate::{corpus::Corpus, Rng};

/// REDQUEEN-style input-to-state keywords (SystemVerilog).
pub static KEYWORD_LIST: &[&str] = &[
    "module", "endmodule", "input", "output", "inout", "wire", "reg",
    "logic", "bit", "integer", "real", "string", "byte", "shortint", "int",
    "longint", "unsigned", "signed", "parameter", "localparam",
    "assign", "always", "always_ff", "always_comb", "always_latch",
    "initial", "final", "begin", "end", "if", "else", "case", "casez",
    "casex", "endcase", "default", "for", "foreach", "while", "do",
    "forever", "repeat", "break", "continue", "return",
    "function", "endfunction", "task", "endtask",
    "class", "endclass", "extends", "implements", "interface", "endinterface",
    "package", "endpackage", "import", "export",
    "struct", "union", "enum", "typedef",
    "generate", "endgenerate", "genvar",
    "assert", "assume", "cover", "property", "sequence", "endproperty",
    "constraint", "rand", "randc", "randomize",
    "fork", "join", "join_any", "join_none",
    "posedge", "negedge", "clocking", "#",
    "and", "or", "not", "xor", "xnor", "nand", "nor",
];

pub struct Mutator<'r> {
    rng: &'r mut Rng,
}

impl<'r> Mutator<'r> {
    pub fn new(rng: &'r mut Rng) -> Self {
        Self { rng }
    }

    /// Terapkan 1 operasi mutasi random (12 opsi).
    pub fn mutate(&mut self, source: &str, corpus: &Corpus) -> String {
        if source.is_empty() {
            return String::new();
        }
        match self.rng.below(12) {
            0 => self.splice_from_corpus(source, corpus),
            1 => self.replace_keyword(source),
            2 => self.delete_chunk(source),
            3 => self.duplicate_chunk(source),
            4 => self.tweak_literal(source),
            5 => self.insert_garbage(source),
            6 => self.remove_line(source),
            7 => self.flip_char(source),
            _ => source.to_string(), // no-op
        }
    }

    /// LangFuzz-style splice: ambil potongan seed donor, sisipkan ke source.
    fn splice_from_corpus(&mut self, source: &str, corpus: &Corpus) -> String {
        let Some(donor) = corpus.random_seed(self.rng) else {
            return source.to_string();
        };
        if donor.is_empty() {
            return source.to_string();
        }
        // Potongan karakter (bukan byte) untuk boundary safety
        let src_chars: Vec<char> = source.chars().collect();
        let donor_chars: Vec<char> = donor.chars().collect();
        if src_chars.is_empty() || donor_chars.is_empty() {
            return source.to_string();
        }

        let cut_len = self.rng.below(donor_chars.len().min(source.len()) / 2 + 1);
        if cut_len == 0 {
            return source.to_string();
        }
        let donor_start = self.rng.below(donor_chars.len() - cut_len.min(donor_chars.len()));
        let frag: String = donor_chars[donor_start..donor_start + cut_len].iter().collect();

        let insert_at = self.rng.below(src_chars.len());
        let mut out: String = src_chars[..insert_at].iter().collect();
        out.push_str(&frag);
        out.extend(&src_chars[insert_at..]);
        out
    }

    /// Ganti keyword SV dengan keyword lain.
    fn replace_keyword(&mut self, source: &str) -> String {
        let words = KEYWORD_LIST;
        let from = self.rng.pick(words);
        let to = self.rng.pick(words);
        let mut out = String::with_capacity(source.len());
        let mut rest = source;
        while let Some(pos) = rest.find(from) {
            out.push_str(&rest[..pos]);
            out.push_str(to);
            rest = &rest[pos + from.len()..];
        }
        out.push_str(rest);
        out
    }

    /// Hapus chunk baris acak.
    fn delete_chunk(&mut self, source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        if lines.len() < 2 {
            return source.to_string();
        }
        let start = self.rng.below(lines.len());
        let end = (start + 1 + self.rng.below(lines.len() - start)).min(lines.len());
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i < start || i >= end {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// Duplikat chunk baris acak.
    fn duplicate_chunk(&mut self, source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        if lines.is_empty() {
            return source.to_string();
        }
        let start = self.rng.below(lines.len());
        let end = (start + 1 + self.rng.below(lines.len() - start)).min(lines.len());
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            out.push_str(line);
            out.push('\n');
            if i == end - 1 {
                // sisipkan salinan chunk di sini
                for j in start..end {
                    out.push_str(lines[j]);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Ubah literal angka — sisipkan nilai ekstrem.
    fn tweak_literal(&mut self, source: &str) -> String {
        let extremes = [
            "'0", "'1", "'x", "'z",
            "32'hffffffff", "64'd0", "-1",
            "1e308", "2**31", "18446744073709551615",
        ];
        let (mut num_start, mut num_end) = (0usize, 0usize);
        let mut in_number = false;
        let mut chosen = None;

        for (i, c) in source.char_indices() {
            if c.is_ascii_digit() || c == '\'' || c == 'x' || c == 'h' || c == 'b' || c == '_' {
                if !in_number {
                    num_start = i;
                    in_number = true;
                }
            } else if in_number {
                if self.rng.chance(10) {
                    chosen = Some((num_start, i));
                    break;
                }
                in_number = false;
            }
            num_end = i;
        }
        if in_number && chosen.is_none() && self.rng.chance(10) {
            chosen = Some((num_start, num_end + 1));
        }

        let Some((s, e)) = chosen else {
            return source.to_string();
        };
        let mut out = String::with_capacity(source.len() + 8);
        out.push_str(&source[..s]);
        out.push_str(self.rng.pick(&extremes));
        out.push_str(&source[e..]);
        out
    }

    /// Sisipkan sampah (garbage) di posisi acak.
    fn insert_garbage(&mut self, source: &str) -> String {
        let garbage = [
            "{", "::", "&&&", "'''", "é中", "@@@", "~~~", "/*",
            "*/", "`define X Y", "`include \"fz.sv\"",
            "endmodule", "begin end", "0x", "##1",
        ];
        let chars: Vec<char> = source.chars().collect();
        if chars.is_empty() {
            return source.to_string();
        }
        let at = self.rng.below(chars.len() + 1);
        let g = self.rng.pick(&garbage);
        let mut out: String = chars[..at].iter().collect();
        out.push_str(g);
        out.extend(&chars[at..]);
        out
    }

    /// Hapus satu baris acak.
    fn remove_line(&mut self, source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        if lines.len() < 2 {
            return source.to_string();
        }
        let idx = self.rng.below(lines.len());
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i != idx {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// Balik satu karakter ascii acak.
    fn flip_char(&mut self, source: &str) -> String {
        let chars: Vec<char> = source.chars().collect();
        if chars.is_empty() {
            return String::new();
        }
        let at = self.rng.below(chars.len());
        let flip_table = ['0', '1', ';', '(', ')', '"', '\'', '+', '-', '&', '|', '^', '~', '!', '=', '<', '>'];
        let new_char = self.rng.pick(&flip_table);
        let mut out: String = chars[..at].iter().collect();
        out.push(*new_char);
        out.extend(&chars[at + 1..]);
        out
    }
}
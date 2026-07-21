use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Module, Endmodule, Input, Output, Inout, Ref,
    Wire, Reg, Logic, Int, Integer, Signed, Unsigned,
    Wand, Wor, Tri, Tri0, Tri1, TriAnd, TriOr,
    Supply0, Supply1,
    Always, AlwaysComb, AlwaysFF, AlwaysLatch,
    Initial, Final, Assign, Begin, End,
    If, Else, Case, CaseX, CaseZ, Endcase,
    For, While, Do, Repeat, Forever,
    PosEdge, NegEdge, Or,
    Param, Parameter, LocalParam,
    GenVar, Generate, EndGenerate,
    Function, EndFunction, Task, EndTask,
    Foreach,
    Auto, Static,
    Real, Time, RealTime,
    String,
    Class, EndClass, Virtual, Extends, This, New, Void,
    Break, Continue,
    Default, Disable, Force, Release, Deassign, Return, Wait,
    Null,
    None, Some_,
    And, Xor, Nand, Nor, Xnor, Buf, NotGate,
    Module_, Interface, EndInterface, ModPort,
    Program, EndProgram,
    Fork, Join, JoinAny, JoinNone,
    Bit, Enum, Typedef, Byte, Shortint, Longint, Struct, Union, EndEnum,
    // Multi-character operators
    Arrow, // ->
    BiDirArrow, // <->
    StarArrow, // *>
    // Literals
    Number { value: String, base: Option<u8>, width: Option<usize>, is_signed: bool },
    RealNum(String),
    StringLit(String),
    Ident(String),

    // Operators
    Plus, Minus, Star, Slash, Percent,
    Equiv, // ===
    NotEquiv, // !==
    CaseEq, // ==?
    CaseNeq, // !=?
    Eq, Neq, Lt, Le, Gt, Ge,
    Tilde, Not, Amp, Pipe, Caret,
    TildeAmp, // ~&
    TildePipe, // ~|
    CaretTilde, // ^~ or ~^
    AmpAmp, PipePipe,
    Shl, Shr, Sshl, Sshr, // <<, >>, <<<, >>>
    PlusColon, MinusColon, // +:, -:
    StarStar, // **

    // Increment / Decrement
    Increment, // ++
    Decrement, // --
    // Assignment
    AssignOp, // =
    PlusAssign, MinusAssign, XorAssign,
    // Blocking / Non-blocking
    BlockingAssign, // =
    NonBlockingAssign, // <=

    // Punctuation
    LParen, RParen, LBrace, RBrace,
    LBrack, RBrack,
    Semi, Comma, Colon, Scope, // ::
    Dot, Hash,
    At, Dollar,
    Question, // ?
    // SystemVerilog-specific
    WildcardEq, // ==*
    WildcardNeq, // !=*

    // SystemVerilog keywords
    Inside, Unique, Priority, Unique0,
    Rand, RandC, Constraint, Const, Var, Solve,
    Assert, Assume, Cover, Expect, WaitOrder, Property,
    Sequence, EndSequence,
    // Package
    Package, EndPackage, Import, Export,
    Mailbox, Semaphore,
    // Bind
    Bind,
    // Specify
    Specify, EndSpecify, SpecParam,
    // Clocking
    Clocking, EndClocking,
    // Config
    Config, EndConfig, Design, Liblist, Cell, Use, Instance,
    // Coverage
    Covergroup, EndGroup, Coverpoint, Cross, Bins, IllegalBins, IgnoreBins, Option_,
    // UDP
    Primitive, EndPrimitive, Table, EndTable,
    // Parameter type
    Type,
    // Special
    FillLit(crate::ir::LogicVal),
    Quote,  // bare ' (for type casts: int'(x))
    Error(String),
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Module => write!(f, "module"),
            Token::Endmodule => write!(f, "endmodule"),
            Token::Input => write!(f, "input"),
            Token::Output => write!(f, "output"),
            Token::Inout => write!(f, "inout"),
            Token::Ref => write!(f, "ref"),
            Token::Wire => write!(f, "wire"),
            Token::Wand => write!(f, "wand"),
            Token::Wor => write!(f, "wor"),
            Token::Tri => write!(f, "tri"),
            Token::Tri0 => write!(f, "tri0"),
            Token::Tri1 => write!(f, "tri1"),
            Token::TriAnd => write!(f, "triand"),
            Token::TriOr => write!(f, "trior"),
            Token::Supply0 => write!(f, "supply0"),
            Token::Supply1 => write!(f, "supply1"),
            Token::Reg => write!(f, "reg"),
            Token::Logic => write!(f, "logic"),
            Token::Int => write!(f, "int"),
            Token::Integer => write!(f, "integer"),
            Token::Signed => write!(f, "signed"),
            Token::Unsigned => write!(f, "unsigned"),
            Token::Always => write!(f, "always"),
            Token::AlwaysComb => write!(f, "always_comb"),
            Token::AlwaysFF => write!(f, "always_ff"),
            Token::AlwaysLatch => write!(f, "always_latch"),
            Token::Initial => write!(f, "initial"),
            Token::Final => write!(f, "final"),
            Token::Assign => write!(f, "assign"),
            Token::Begin => write!(f, "begin"),
            Token::End => write!(f, "end"),
            Token::If => write!(f, "if"),
            Token::Else => write!(f, "else"),
            Token::Case => write!(f, "case"),
            Token::CaseX => write!(f, "casex"),
            Token::CaseZ => write!(f, "casez"),
            Token::Endcase => write!(f, "endcase"),
            Token::For => write!(f, "for"),
            Token::Foreach => write!(f, "foreach"),
            Token::While => write!(f, "while"),
            Token::Do => write!(f, "do"),
            Token::Repeat => write!(f, "repeat"),
            Token::Forever => write!(f, "forever"),
            Token::Fork => write!(f, "fork"),
            Token::Join => write!(f, "join"),
            Token::JoinAny => write!(f, "join_any"),
            Token::JoinNone => write!(f, "join_none"),
            Token::PosEdge => write!(f, "posedge"),
            Token::NegEdge => write!(f, "negedge"),
            Token::Or => write!(f, "or"),
            Token::Param => write!(f, "param"),
            Token::Parameter => write!(f, "parameter"),
            Token::LocalParam => write!(f, "localparam"),
            Token::GenVar => write!(f, "genvar"),
            Token::Generate => write!(f, "generate"),
            Token::EndGenerate => write!(f, "endgenerate"),
            Token::Default => write!(f, "default"),
            Token::Disable => write!(f, "disable"),
            Token::Force => write!(f, "force"),
            Token::Release => write!(f, "release"),
            Token::Deassign => write!(f, "deassign"),
            Token::Return => write!(f, "return"),
            Token::Wait => write!(f, "wait"),
            Token::Scope => write!(f, "::"),
            Token::Quote => write!(f, "'"),
            Token::Increment => write!(f, "++"),
            Token::Decrement => write!(f, "--"),
            Token::Inside => write!(f, "inside"),
            Token::Rand => write!(f, "rand"),
            Token::RandC => write!(f, "randc"),
            Token::Constraint => write!(f, "constraint"),
            Token::Const => write!(f, "const"),
            Token::Var => write!(f, "var"),
            Token::Solve => write!(f, "solve"),
            Token::Unique => write!(f, "unique"),
            Token::Priority => write!(f, "priority"),
            Token::Unique0 => write!(f, "unique0"),
            Token::Assert => write!(f, "assert"),
            Token::Assume => write!(f, "assume"),
            Token::Cover => write!(f, "cover"),
            Token::Expect => write!(f, "expect"),
            Token::WaitOrder => write!(f, "wait_order"),
            Token::Property => write!(f, "property"),
            Token::Sequence => write!(f, "sequence"),
            Token::EndSequence => write!(f, "endsequence"),
            Token::Package => write!(f, "package"),
            Token::EndPackage => write!(f, "endpackage"),
            Token::Import => write!(f, "import"),
            Token::Export => write!(f, "export"),
            Token::Bind => write!(f, "bind"),
            Token::Specify => write!(f, "specify"),
            Token::EndSpecify => write!(f, "endspecify"),
            Token::SpecParam => write!(f, "specparam"),
            Token::Clocking => write!(f, "clocking"),
            Token::EndClocking => write!(f, "endclocking"),
            Token::Config => write!(f, "config"),
            Token::EndConfig => write!(f, "endconfig"),
            Token::Design => write!(f, "design"),
            Token::Liblist => write!(f, "liblist"),
            Token::Cell => write!(f, "cell"),
            Token::Use => write!(f, "use"),
            Token::Instance => write!(f, "instance"),
            Token::Primitive => write!(f, "primitive"),
            Token::EndPrimitive => write!(f, "endprimitive"),
            Token::Table => write!(f, "table"),
            Token::EndTable => write!(f, "endtable"),
            Token::Covergroup => write!(f, "covergroup"),
            Token::EndGroup => write!(f, "endgroup"),
            Token::Coverpoint => write!(f, "coverpoint"),
            Token::Cross => write!(f, "cross"),
            Token::Bins => write!(f, "bins"),
            Token::IllegalBins => write!(f, "illegal_bins"),
            Token::IgnoreBins => write!(f, "ignore_bins"),
            Token::Option_ => write!(f, "option"),
            Token::Type => write!(f, "type"),
            Token::Program => write!(f, "program"),
            Token::EndProgram => write!(f, "endprogram"),
            Token::Eof => write!(f, "<eof>"),
            Token::Error(s) => write!(f, "<error: {}>", s),
            _ => write!(f, "{:?}", self),
        }
    }
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        Self { chars, pos: 0, line: 1, col: 1 }
    }

    pub fn next_token(&mut self) -> (Token, usize, usize) {
        self.skip_whitespace_and_comments();
        if self.pos >= self.chars.len() {
            return (Token::Eof, self.line, self.col);
        }

        let c = self.chars[self.pos];
        let start_line = self.line;
        let start_col = self.col;

        // Identifier or keyword
        if c.is_ascii_alphabetic() || c == '_' || c == '\\' {
            let (tok, _, _) = self.read_ident_or_keyword();
            return (tok, start_line, start_col);
        }

        // Number literal
        if c.is_ascii_digit() {
            let tok = self.read_number();
            return (tok, start_line, start_col);
        }

        // String literal
        if c == '"' {
            let tok = self.read_string();
            return (tok, start_line, start_col);
        }

        // Operators and punctuation
        let tok = self.read_operator_or_punct();
        (tok, start_line, start_col)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> char {
        let c = self.chars[self.pos];
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            self.skip_whitespace();
            if self.pos >= self.chars.len() {
                break;
            }
            // Single-line comment
            if self.peek() == Some('/') && self.peek_next() == Some('/') {
                self.skip_single_line_comment();
                continue;
            }
            // Multi-line comment
            if self.peek() == Some('/') && self.peek_next() == Some('*') {
                self.skip_multi_line_comment();
                continue;
            }
            // `line directive — update line counter
            if self.peek() == Some('`') {
                if self.handle_line_directive() {
                    continue;
                }
                // Unknown backtick directive — skip line silently
                while self.peek().is_some() && self.peek() != Some('\n') {
                    self.advance();
                }
                if self.peek() == Some('\n') {
                    self.advance();
                }
                continue;
            }
            break;
        }
    }

    fn handle_line_directive(&mut self) -> bool {
        let line_start = self.pos;
        let mut line_end = self.pos;
        while line_end < self.chars.len() && self.chars[line_end] != '\n' {
            line_end += 1;
        }
        let line_content: String = self.chars[line_start..line_end].iter().collect();
        if !line_content.trim_start().starts_with("`line") {
            return false;
        }
        let after_cmd = line_content.trim_start()[5..].trim();
        let num_str = if let Some(quote_pos) = after_cmd.find('"') {
            after_cmd[..quote_pos].trim()
        } else {
            after_cmd.trim()
        };
        if let Ok(new_line) = num_str.parse::<usize>() {
            self.line = new_line;
            self.pos = line_end;
            if self.pos < self.chars.len() && self.chars[self.pos] == '\n' {
                self.pos += 1;
            }
            self.col = 1;
            return true;
        }
        false
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_single_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                self.advance();
                break;
            }
            self.advance();
        }
    }

    fn skip_multi_line_comment(&mut self) {
        self.advance(); // /
        self.advance(); // *
        loop {
            if self.pos >= self.chars.len() {
                break;
            }
            if self.peek() == Some('*') && self.peek_next() == Some('/') {
                self.advance();
                self.advance();
                break;
            }
            self.advance();
        }
    }

    fn read_ident_or_keyword(&mut self) -> (Token, usize, usize) {
        let mut s = String::new();
        let start_line = self.line;
        let start_col = self.col;

        if self.peek() == Some('\\') {
            self.advance(); // consume backslash
            while let Some(c) = self.peek() {
                if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                    self.advance(); // consume trailing whitespace
                    break;
                }
                s.push(self.advance());
            }
            return (Token::Ident(s), start_line, start_col);
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                s.push(self.advance());
            } else {
                break;
            }
        }

        let token = match s.as_str() {
            "module" => Token::Module,
            "endmodule" => Token::Endmodule,
            "input" => Token::Input,
            "output" => Token::Output,
            "inout" => Token::Inout,
            "ref" => Token::Ref,
            "wire" => Token::Wire,
            "wand" => Token::Wand,
            "wor" => Token::Wor,
            "tri" => Token::Tri,
            "tri0" => Token::Tri0,
            "tri1" => Token::Tri1,
            "triand" => Token::TriAnd,
            "trior" => Token::TriOr,
            "supply0" => Token::Supply0,
            "supply1" => Token::Supply1,
            "reg" => Token::Reg,
            "logic" => Token::Logic,
            "int" => Token::Int,
            "integer" => Token::Integer,
            "signed" => Token::Signed,
            "unsigned" => Token::Unsigned,
            "always" => Token::Always,
            "always_comb" => Token::AlwaysComb,
            "always_ff" => Token::AlwaysFF,
            "always_latch" => Token::AlwaysLatch,
            "initial" => Token::Initial,
            "final" => Token::Final,
            "assign" => Token::Assign,
            "begin" => Token::Begin,
            "end" => Token::End,
            "if" => Token::If,
            "else" => Token::Else,
            "case" => Token::Case,
            "casex" => Token::CaseX,
            "casez" => Token::CaseZ,
            "endcase" => Token::Endcase,
            "for" => Token::For,
            "foreach" => Token::Foreach,
            "while" => Token::While,
            "do" => Token::Do,
            "repeat" => Token::Repeat,
            "forever" => Token::Forever,
            "fork" => Token::Fork,
            "join" => Token::Join,
            "join_any" => Token::JoinAny,
            "join_none" => Token::JoinNone,
            "posedge" => Token::PosEdge,
            "negedge" => Token::NegEdge,
            "or" => Token::Or,
            "param" => Token::Param,
            "parameter" => Token::Parameter,
            "localparam" => Token::LocalParam,
            "genvar" => Token::GenVar,
            "generate" => Token::Generate,
            "endgenerate" => Token::EndGenerate,
            "default" => Token::Default,
            "disable" => Token::Disable,
            "force" => Token::Force,
            "release" => Token::Release,
            "deassign" => Token::Deassign,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "wait" => Token::Wait,
            "null" => Token::Null,
            "string" => Token::String,
            "mailbox" => Token::Mailbox,
            "semaphore" => Token::Semaphore,
            "function" => Token::Function,
            "endfunction" => Token::EndFunction,
            "task" => Token::Task,
            "endtask" => Token::EndTask,
            "automatic" | "auto" => Token::Auto,
            "static" => Token::Static,
            "real" => Token::Real,
            "time" => Token::Time,
            "realtime" => Token::RealTime,
            "none" => Token::None,
            "some" => Token::Some_,
            "and" => Token::And,
            "xor" => Token::Xor,
            "nand" => Token::Nand,
            "nor" => Token::Nor,
            "xnor" => Token::Xnor,
            "buf" => Token::Buf,
            "not" => Token::NotGate,
            "class" => Token::Class,
            "endclass" => Token::EndClass,
            "virtual" => Token::Virtual,
            "extends" => Token::Extends,
            "this" => Token::This,
            "new" => Token::New,
            "void" => Token::Void,
            "return" => Token::Return,
            "enum" => Token::Enum,
            "type" => Token::Type,
            "typedef" => Token::Typedef,
            "bit" => Token::Bit,
            "byte" => Token::Byte,
            "shortint" => Token::Shortint,
            "longint" => Token::Longint,
            "struct" => Token::Struct,
            "union" => Token::Union,
            "endenum" => Token::EndEnum,
            "modport" => Token::ModPort,
            "program" => Token::Program,
            "endprogram" => Token::EndProgram,
            "interface" => Token::Interface,
            "endinterface" => Token::EndInterface,
            "rand" => Token::Rand,
            "randc" => Token::RandC,
            "constraint" => Token::Constraint,
            "const" => Token::Const,
            "var" => Token::Var,
            "solve" => Token::Solve,
            "inside" => Token::Inside,
            "unique" => Token::Unique,
            "priority" => Token::Priority,
            "unique0" => Token::Unique0,
            "assert" => Token::Assert,
            "assume" => Token::Assume,
            "cover" => Token::Cover,
            "expect" => Token::Expect,
            "wait_order" => Token::WaitOrder,
            "property" => Token::Property,
            "sequence" => Token::Sequence,
            "endsequence" => Token::EndSequence,
            "package" => Token::Package,
            "endpackage" => Token::EndPackage,
            "import" => Token::Import,
            "export" => Token::Export,
            "bind" => Token::Bind,
            "specify" => Token::Specify,
            "endspecify" => Token::EndSpecify,
            "specparam" => Token::SpecParam,
            "clocking" => Token::Clocking,
            "endclocking" => Token::EndClocking,
            "config" => Token::Config,
            "endconfig" => Token::EndConfig,
            "design" => Token::Design,
            "liblist" => Token::Liblist,
            "cell" => Token::Cell,
            "use" => Token::Use,
            "instance" => Token::Instance,
            "covergroup" => Token::Covergroup,
            "endgroup" => Token::EndGroup,
            "primitive" => Token::Primitive,
            "endprimitive" => Token::EndPrimitive,
            "table" => Token::Table,
            "endtable" => Token::EndTable,
            "coverpoint" => Token::Coverpoint,
            "cross" => Token::Cross,
            "bins" => Token::Bins,
            "illegal_bins" => Token::IllegalBins,
            "ignore_bins" => Token::IgnoreBins,
            "option" => Token::Option_,
            _ => Token::Ident(s),
        };

        (token, start_line, start_col)
    }

    fn read_number(&mut self) -> Token {

        // Or simple decimal
        let mut s = String::new();

        // Collect all digits (possibly with underscores)
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' || c == 'x' || c == 'z' || c == 'X' || c == 'Z' {
                s.push(self.advance());
        } else if c == '\'' {
            // Check for size cast: 22'(expr) — next char after ' should not be '('
            if self.peek_next() == Some('(') {
                // This is a cast: 22'(expr). Leave ' for the next token (Quote).
                break;
            }
            // Could be sized format like 8'b1010
            s.push(self.advance());
            // Read base character
            if self.peek().is_some() {
                s.push(self.advance());
            }
                // Skip whitespace before value (e.g., 32'h 0000_0000)
                while let Some(c) = self.peek() {
                    if c.is_ascii_whitespace() {
                        self.advance();
                    } else {
                        break;
                    }
                }
                // Read the value part
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' || c == 'x' || c == 'z' || c == 'X' || c == 'Z' || c == '?' {
                        s.push(self.advance());
                    } else {
                        break;
                    }
                }
                return self.parse_verilog_number(&s);
            } else {
                break;
            }
        }

        // Check for real number (contains decimal point)
        if self.peek() == Some('.') {
            s.push(self.advance());
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '_' {
                    s.push(self.advance());
                } else {
                    break;
                }
            }
            // Check for exponent
            if self.peek() == Some('e') || self.peek() == Some('E') {
                s.push(self.advance());
                if self.peek() == Some('+') || self.peek() == Some('-') {
                    s.push(self.advance());
                }
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        s.push(self.advance());
                    } else {
                        break;
                    }
                }
            }
            return Token::RealNum(s);
        }

        // Plain decimal
        Token::Number { value: s, base: None, width: None, is_signed: false }
    }

    fn parse_verilog_number(&self, s: &str) -> Token {
        // Format: [width]'[s]base value
        // e.g., 8'b10101010, 32'habcd, '1, '0
        let parts: Vec<&str> = s.split('\'').collect();
        if parts.len() != 2 {
            return Token::Number { value: s.to_string(), base: None, width: None, is_signed: false };
        }

        let width_str = parts[0];
        let width = if width_str.is_empty() {
            None
        } else {
            width_str.replace('_', "").parse::<usize>().ok()
        };

        let rest = parts[1];
        if rest.is_empty() {
            return Token::Number { value: s.to_string(), base: None, width: None, is_signed: false };
        }

        let mut chars = rest.chars();
        let first_char = chars.next().unwrap();
        let (base_char, is_signed) = match first_char {
            's' | 'S' => (chars.next().unwrap_or('d'), true),
            _ => (first_char, false),
        };
        let value_part: String = chars.collect();

        let base = match base_char {
            'b' | 'B' => Some(2),
            'o' | 'O' => Some(8),
            'd' | 'D' => Some(10),
            'h' | 'H' => Some(16),
            _ => None,
        };

        Token::Number {
            value: value_part.replace('_', "").replace('?', "z"),
            base,
            width,
            is_signed,
        }
    }

    fn read_string(&mut self) -> Token {
        let mut s = String::new();
        self.advance(); // consume opening "
        while let Some(c) = self.peek() {
            if c == '"' {
                self.advance(); // consume closing "
                break;
            }
            if c == '\\' {
                self.advance();
                if let Some(esc) = self.peek() {
                    match esc {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        other => { s.push('\\'); s.push(other); }
                    }
                    self.advance();
                }
            } else {
                s.push(self.advance());
            }
        }
        Token::StringLit(s)
    }

    fn read_operator_or_punct(&mut self) -> Token {
        let c = self.advance();
        match c {
            '+' => {
                if self.peek() == Some('+') { self.advance(); Token::Increment }
                else if self.peek() == Some('=') { self.advance(); Token::PlusAssign }
                else if self.peek() == Some(':') { self.advance(); Token::PlusColon }
                else { Token::Plus }
            }
            '-' => {
                if self.peek() == Some('-') { self.advance(); Token::Decrement }
                else if self.peek() == Some('>') { self.advance(); Token::Arrow } // ->
                else if self.peek() == Some(':') { self.advance(); Token::MinusColon }
                else { Token::Minus }
            }
            '*' => {
                if self.peek() == Some('*') { self.advance(); Token::StarStar }
                else if self.peek() == Some('>') { self.advance(); Token::StarArrow } // *>
                else { Token::Star }
            }
            '/' => Token::Slash,
            '%' => Token::Percent,
            '=' => {
                if self.peek() == Some('=') && self.peek_next() == Some('=') {
                    // ===
                    self.advance(); self.advance();
                    Token::Equiv
                }
                else if self.peek() == Some('=') && self.peek_next() == Some('?') {
                    // ==?
                    self.advance(); self.advance();
                    Token::CaseEq
                }
                else if self.peek() == Some('=') { self.advance(); Token::Eq } // ==
                else if self.peek() == Some('*') { self.advance(); Token::WildcardEq } // ==*
                else { Token::BlockingAssign } // =
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    if self.peek() == Some('<') { self.advance(); Token::Sshl } // <=<
                    else { Token::NonBlockingAssign } // <=
                }
                else if self.peek() == Some('<') {
                    self.advance();
                    if self.peek() == Some('<') { self.advance(); Token::Sshl } // <<<
                    else { Token::Shl }
                }
                else if self.peek() == Some('-') {
                    self.advance();
                    if self.peek() == Some('>') { self.advance(); Token::BiDirArrow } // <->
                    else { Token::Minus /* not really, but keep as fallback */ }
                }
                else { Token::Lt }
            }
            '>' => {
                if self.peek() == Some('=') { self.advance(); Token::Ge }
                else if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') { self.advance(); Token::Sshr } // >>>
                    else { Token::Shr }
                }
                else { Token::Gt }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); Token::NotEquiv } // !==
                    else if self.peek() == Some('?') { self.advance(); Token::CaseNeq } // !=?
                    else if self.peek() == Some('*') { self.advance(); Token::WildcardNeq } // !=*
                    else { Token::Neq }
                }
                else { Token::Not }
            }
            '~' => {
                if self.peek() == Some('&') { self.advance(); Token::TildeAmp }
                else if self.peek() == Some('|') { self.advance(); Token::TildePipe }
                else if self.peek() == Some('^') { self.advance(); Token::CaretTilde }
                else { Token::Tilde }
            }
            '&' => {
                if self.peek() == Some('&') { self.advance(); Token::AmpAmp }
                else { Token::Amp }
            }
            '|' => {
                if self.peek() == Some('|') { self.advance(); Token::PipePipe }
                else { Token::Pipe }
            }
            '^' => {
                if self.peek() == Some('~') { self.advance(); Token::CaretTilde }
                else if self.peek() == Some('=') { self.advance(); Token::XorAssign }
                else { Token::Caret }
            }
            ':' => {
                if self.peek() == Some(':') { self.advance(); Token::Scope } // ::
                else if self.peek() == Some('=') { self.advance(); Token::AssignOp } // :=
                else { Token::Colon }
            }
            '?' => Token::Question,
            '(' => Token::LParen,
            ')' => Token::RParen,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '[' => Token::LBrack,
            ']' => Token::RBrack,
            ';' => Token::Semi,
            ',' => Token::Comma,
            '.' => Token::Dot,
            '#' => Token::Hash,
            '@' => Token::At,
            '$' => Token::Dollar,
            '\'' => {
                let next = self.peek();
                match next {
                    Some('0') => { self.advance(); Token::FillLit(crate::ir::LogicVal::Zero) }
                    Some('1') => { self.advance(); Token::FillLit(crate::ir::LogicVal::One) }
                    Some('x') | Some('X') => { self.advance(); Token::FillLit(crate::ir::LogicVal::X) }
                    Some('z') | Some('Z') => { self.advance(); Token::FillLit(crate::ir::LogicVal::Z) }
                    // Unsized literals: 'b, 'o, 'd, 'h (without leading digit)
                    Some('b') | Some('B') | Some('o') | Some('O') | Some('d') | Some('D') | Some('h') | Some('H') => {
                        let base_char = next.unwrap();
                        self.advance();
                        let base = match base_char { 'b'|'B' => 2, 'o'|'O' => 8, 'd'|'D' => 10, _ => 16 };
                        let mut value = String::new();
                        while let Some(c) = self.peek() {
                            if c.is_ascii_alphanumeric() || c == '_' || c == 'x' || c == 'z' || c == 'X' || c == 'Z' {
                                value.push(self.advance());
                            } else { break; }
                        }
                        Token::Number { value: value.replace('_', ""), base: Some(base), width: None, is_signed: false }
                    }
                    // Unsized signed literals: 'sb, 'sd, 'sh, 'so
                    Some('s') | Some('S') => {
                        self.advance();
                        match self.peek() {
                            Some(c @ ('b'|'B'|'o'|'O'|'d'|'D'|'h'|'H')) => {
                                self.advance();
                                let base = match c { 'b'|'B' => 2, 'o'|'O' => 8, 'd'|'D' => 10, _ => 16 };
                                let mut value = String::new();
                                while let Some(c) = self.peek() {
                                    if c.is_ascii_alphanumeric() || c == '_' || c == 'x' || c == 'z' || c == 'X' || c == 'Z' {
                                        value.push(self.advance());
                                    } else { break; }
                                }
                                Token::Number { value: value.replace('_', ""), base: Some(base), width: None, is_signed: true }
                            }
                            _ => Token::Error(format!("expected base after 's in literal")),
                        }
                    }
                    _ => Token::Quote,
                }
            }
            _ => Token::Error(format!("unexpected character '{}'", c)),
        }
    }
}

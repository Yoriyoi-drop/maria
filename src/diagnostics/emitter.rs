//! Diagnostic Emitter — formatted output untuk terminal dan LSP.

use std::io::{self, Write};

use super::diagnostic::{DiagLevel, DiagSink, Diagnostic};

/// Terminal emitter — format diagnostics untuk console output.
pub struct TerminalEmitter {
    writer: Box<dyn Write + Send>,
    use_color: bool,
}

impl TerminalEmitter {
    pub fn new() -> Self {
        TerminalEmitter {
            writer: Box::new(io::stderr()),
            use_color: atty_is_terminal(),
        }
    }

    pub fn with_writer(writer: Box<dyn Write + Send>) -> Self {
        TerminalEmitter {
            writer,
            use_color: false,
        }
    }

    pub fn with_color(mut self, use_color: bool) -> Self {
        self.use_color = use_color;
        self
    }

    /// Emit a single diagnostic.
    pub fn emit(&mut self, diag: &Diagnostic) -> io::Result<()> {
        if self.use_color {
            self.emit_colored(diag)
        } else {
            self.emit_plain(diag)
        }
    }

    fn emit_colored(&mut self, diag: &Diagnostic) -> io::Result<()> {
        let color = match diag.level {
            DiagLevel::Bug => "\x1b[35m",     // magenta
            DiagLevel::Error => "\x1b[31m",   // red
            DiagLevel::Warning => "\x1b[33m", // yellow
            DiagLevel::Note => "\x1b[36m",    // cyan
            DiagLevel::Help => "\x1b[32m",    // green
        };
        let reset = "\x1b[0m";

        writeln!(
            self.writer,
            "{}{}{}: {}: {}",
            color, diag.level, reset, diag.code, diag.message
        )?;

        for span in &diag.spans {
            write!(
                self.writer,
                "  {}-->{} {}:{}:{}",
                "\x1b[34m", reset, span.file, span.start, span.end
            )?;
            if let Some(label) = &span.label {
                write!(self.writer, " — {}", label)?;
            }
            writeln!(self.writer)?;
        }

        for note in &diag.notes {
            writeln!(
                self.writer,
                "  {}note:{} {}",
                "\x1b[36m", reset, note.message
            )?;
        }

        for hint in &diag.hints {
            writeln!(self.writer, "  {}help:{} {}", "\x1b[32m", reset, hint)?;
        }

        Ok(())
    }

    fn emit_plain(&mut self, diag: &Diagnostic) -> io::Result<()> {
        writeln!(
            self.writer,
            "{}: {}: {}",
            diag.level, diag.code, diag.message
        )?;

        for span in &diag.spans {
            write!(
                self.writer,
                "  --> {}:{}:{}",
                span.file, span.start, span.end
            )?;
            if let Some(label) = &span.label {
                write!(self.writer, " — {}", label)?;
            }
            writeln!(self.writer)?;
        }

        for note in &diag.notes {
            writeln!(self.writer, "  = note: {}", note.message)?;
        }

        for hint in &diag.hints {
            writeln!(self.writer, "  = help: {}", hint)?;
        }

        Ok(())
    }

    /// Emit all diagnostics from a sink.
    pub fn emit_all(&mut self, sink: &DiagSink) -> io::Result<usize> {
        let diags = sink.diagnostics();
        let count = diags.len();
        for diag in &diags {
            self.emit(diag)?;
        }
        Ok(count)
    }
}

impl Default for TerminalEmitter {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if stderr is a terminal (for color support).
fn atty_is_terminal() -> bool {
    #[cfg(unix)]
    {
        // Use simple check via stdio
        use std::os::unix::io::AsRawFd;
        let fd = std::io::stderr().as_raw_fd();
        libc_isatty(fd) != 0
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Minimal isatty wrapper (avoid libc dependency).
#[cfg(unix)]
fn libc_isatty(fd: i32) -> i32 {
    unsafe extern "C" {
        fn isatty(fd: i32) -> i32;
    }
    unsafe { isatty(fd) }
}

/// Format diagnostic as a single string (for logging/testing).
pub fn format_diagnostic(diag: &Diagnostic) -> String {
    let mut output = format!("{}: {}: {}\n", diag.level, diag.code, diag.message);

    for span in &diag.spans {
        output.push_str(&format!("  --> {}:{}:{}", span.file, span.start, span.end));
        if let Some(label) = &span.label {
            output.push_str(&format!(" — {}", label));
        }
        output.push('\n');
    }

    for note in &diag.notes {
        output.push_str(&format!("  = note: {}\n", note.message));
    }

    for hint in &diag.hints {
        output.push_str(&format!("  = help: {}\n", hint));
    }

    output
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::diagnostic::{DiagCode, DiagSpan};
    use crate::intern::Symbol;

    #[test]
    fn test_format_diagnostic() {
        let file = Symbol::intern("test.sv");
        let d = Diagnostic::error(DiagCode::UnexpectedToken, "found 'foo' where ';' expected")
            .with_span(DiagSpan::new(file, 10, 13).with_label("here"))
            .with_note("try adding a semicolon");

        let output = format_diagnostic(&d);
        assert!(output.contains("E1001"));
        assert!(output.contains("found 'foo'"));
        assert!(output.contains("test.sv:10:13"));
        assert!(output.contains("note: try adding"));
    }

    #[test]
    fn test_terminal_emitter_plain() {
        let d = Diagnostic::warning(DiagCode::WidthMismatch, "width differs");
        let output = format_diagnostic(&d);
        assert!(output.contains("warning"));
        assert!(output.contains("E2003"));
    }
}

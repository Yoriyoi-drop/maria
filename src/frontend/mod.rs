//! Frontend — file discovery, lexing, parsing, preprocessing.
//! Pipeline paralel untuk kompilasi SystemVerilog.

pub mod compile_session;
pub mod discovery;
pub mod io;
pub mod module_index;
pub mod package_resolver;
pub mod lexer; // byte-level lexer

// Re-export FastLexer at top level
pub use lexer::FastLexer;

pub mod legacy_lexer {
    //! Re-export existing legacy lexer
    pub use crate::parser::lexer::*;
}

pub mod parser {
    //! Re-export existing parser
    pub use crate::parser::parser::*;
}

pub mod preprocessor {
    //! Re-export existing preprocessor
    pub use crate::parser::preprocessor::*;
}

pub use compile_session::CompileSession;
pub use discovery::FileDiscovery;
pub use module_index::ModuleIndex;
pub use package_resolver::PackageResolver;

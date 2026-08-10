//! Verification context — semua checker: lint, semantic, typecheck, xprop,
//! assertions, coverage. Satu arah dari compiler.

mod assertions;
mod coverage;
mod lint;
mod semantic;
mod verification;
mod xprop;

pub use assertions::{AssertionReport, AssertionSummary};
pub use coverage::{CoverageSettings, coverage_summary};
pub use lint::{LintChecks, LintReport};
pub use semantic::SemanticStatus;
pub use verification::VerificationContext;
pub use xprop::{XPropMode, current_xprop, set_xprop};

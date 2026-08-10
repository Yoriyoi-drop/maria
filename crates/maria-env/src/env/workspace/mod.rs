//! Workspace context — mengelola layout project: root path, direktori source
//! (rtl/, tb/, ip/, ...), filelist, incdir, library, output path.

mod filelist;
mod include;
mod project;
mod search;
mod workspace;

pub use filelist::Filelist;
pub use include::IncludeDirs;
pub use project::ProjectFile;
pub use search::{find_in_dirs, resolve_header};
pub use workspace::WorkspaceContext;

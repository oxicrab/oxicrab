//! System tools for the oxicrab framework.
//!
//! This crate provides filesystem, shell, and tmux tools,
//! extracted from the main binary crate for modularity.

pub mod filesystem;
pub mod shell;
pub mod tmux;
mod utils;

use oxicrab_core::config::schema::{AllowedCommands, SandboxConfig};
use oxicrab_core::tools::base::Tool;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Create filesystem tools (read_file, write_file, edit_file, list_dir).
///
/// - `workspace`: working directory for the agent
/// - `roots`: if `Some`, restricts filesystem access to these directories
/// - `backup_dir`: if `Some`, backups are created before writes/edits
pub fn create_filesystem_tools(
    workspace: &Path,
    roots: Option<Vec<PathBuf>>,
    backup_dir: Option<PathBuf>,
) -> Vec<Arc<dyn Tool>> {
    let ws = Some(workspace.to_path_buf());

    let read = filesystem::ReadFileTool::new(roots.clone(), ws.clone());
    let write = filesystem::WriteFileTool::new(roots.clone(), backup_dir.clone(), ws.clone());
    let edit = filesystem::EditFileTool::new(roots.clone(), backup_dir, ws.clone());
    let list = filesystem::ListDirTool::new(roots, ws);

    vec![
        Arc::new(read),
        Arc::new(write),
        Arc::new(edit),
        Arc::new(list),
    ]
}

/// Create the exec (shell) tool.
///
/// Returns `Err` if security patterns fail to compile.
pub fn create_exec_tool(
    timeout: u64,
    working_dir: Option<PathBuf>,
    restrict_to_workspace: bool,
    allowed_commands: AllowedCommands,
    sandbox_config: SandboxConfig,
) -> anyhow::Result<Arc<dyn Tool>> {
    Ok(Arc::new(shell::ExecTool::new(
        timeout,
        working_dir,
        restrict_to_workspace,
        allowed_commands,
        sandbox_config,
    )?))
}

/// Create the tmux tool.
pub fn create_tmux_tool() -> Arc<dyn Tool> {
    Arc::new(tmux::TmuxTool::new())
}

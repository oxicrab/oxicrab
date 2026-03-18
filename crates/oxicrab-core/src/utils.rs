use std::path::PathBuf;

pub fn get_workspace_path(workspace: &str) -> PathBuf {
    if workspace.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            let stripped = workspace.strip_prefix("~/").unwrap_or(workspace);
            return home.join(stripped);
        }
    } else if workspace == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = workspace.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        let relative = rest.strip_prefix('/').unwrap_or(rest);
        return home.join(relative);
    }
    PathBuf::from(workspace)
}

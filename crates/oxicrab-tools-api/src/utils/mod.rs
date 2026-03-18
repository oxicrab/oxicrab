pub(crate) use oxicrab_core::utils::{http, media, truncate_chars};

pub(crate) fn sanitize_path(
    path: &std::path::Path,
    _workspace: Option<&std::path::Path>,
) -> String {
    // Simplified version -- just use tilde collapse
    let path_str = path.to_string_lossy();
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path_str.starts_with(home_str.as_ref()) {
            return format!("~{}", &path_str[home_str.len()..]);
        }
    }
    path_str.to_string()
}

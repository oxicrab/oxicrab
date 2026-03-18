// Re-export from oxicrab-core so all existing imports continue to work.
pub use oxicrab_core::tools::base::routing_types;
pub use oxicrab_core::tools::base::*;

/// Build a `Vec<ActionDescriptor>` concisely.
///
/// Mark read-only actions with `: ro`:
/// ```ignore
/// actions![
///     list_issues: ro,       // read-only action
///     create_issue,          // mutating action (default)
/// ]
/// ```
#[macro_export]
macro_rules! actions {
    (@one $name:ident : ro) => {
        $crate::agent::tools::base::ActionDescriptor { name: stringify!($name), read_only: true }
    };
    (@one $name:ident) => {
        $crate::agent::tools::base::ActionDescriptor { name: stringify!($name), read_only: false }
    };
    ($($name:ident $(: $ro:ident)?),+ $(,)?) => {
        vec![$(actions!(@one $name $(: $ro)?)),+]
    };
}

/// Extract a required string parameter from a JSON `Value`, returning a
/// `ToolResult::error` if the key is missing or not a string.
///
/// Usage: `let action = require_param!(params, "action");`
#[macro_export]
macro_rules! require_param {
    ($params:expr, $key:literal) => {
        match $params[$key].as_str() {
            Some(v) => v,
            None => {
                return Ok($crate::agent::tools::base::ToolResult::error(format!(
                    "Missing '{}' parameter",
                    $key
                )));
            }
        }
    };
}

//! Google tools for the oxicrab framework.
//!
//! This crate provides Gmail, Google Calendar, and Google Tasks tools.

pub mod auth;
pub mod credentials;
pub mod google_calendar;
pub mod google_common;
pub mod google_mail;
pub mod google_tasks;
mod utils;

use credentials::GoogleCredentials;
use oxicrab_core::tools::base::Tool;
use std::sync::Arc;

/// Create Google tools from pre-authenticated credentials.
///
/// The caller is responsible for obtaining the `GoogleCredentials` (e.g., via OAuth flow).
/// Returns a vec of tools based on which services are enabled in the config.
pub fn create_google_tools(
    credentials: GoogleCredentials,
    gmail: bool,
    calendar: bool,
    tasks: bool,
) -> Vec<Arc<dyn Tool>> {
    let mut result: Vec<Arc<dyn Tool>> = Vec::new();

    if gmail {
        result.push(Arc::new(google_mail::GoogleMailTool::new(
            credentials.clone(),
        )));
    }
    if calendar {
        result.push(Arc::new(google_calendar::GoogleCalendarTool::new(
            credentials.clone(),
        )));
    }
    if tasks {
        result.push(Arc::new(google_tasks::GoogleTasksTool::new(credentials)));
    }

    result
}

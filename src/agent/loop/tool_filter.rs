use crate::agent::tools::base::ToolCategory;

/// Skip tool pre-filtering when total tools are at or below this count.
pub(super) const TOOL_FILTER_THRESHOLD: usize = 30;

/// Infer which tool categories are relevant for a user message.
/// Core and System are always included.
pub(super) fn infer_tool_categories(content: &str) -> Vec<ToolCategory> {
    use ToolCategory::{
        Communication, Core, Development, Media, Productivity, Scheduling, System, Web,
    };

    let lower = content.to_lowercase();
    let mut cats = vec![Core, System];

    // Web
    if lower.contains("search")
        || lower.contains("look up")
        || lower.contains("lookup")
        || lower.contains("find online")
        || lower.contains("browse")
        || lower.contains("website")
        || lower.contains("url")
        || lower.contains("http")
        || lower.contains("weather")
        || lower.contains("forecast")
        || lower.contains("fetch")
    {
        cats.push(Web);
    }

    // Communication
    if lower.contains("email")
        || lower.contains("mail")
        || lower.contains("send")
        || lower.contains("inbox")
        || lower.contains("draft")
    {
        cats.push(Communication);
    }

    // Development
    if lower.contains("github")
        || lower.contains("pr")
        || lower.contains("pull request")
        || lower.contains("issue")
        || lower.contains("commit")
        || lower.contains("repo")
        || lower.contains("branch")
        || lower.contains("workflow")
    {
        cats.push(Development);
    }

    // Scheduling
    if lower.contains("schedule")
        || lower.contains("cron")
        || lower.contains("calendar")
        || lower.contains("reminder")
        || lower.contains("event")
        || lower.contains("appointment")
    {
        cats.push(Scheduling);
    }

    // Media
    if lower.contains("image")
        || lower.contains("photo")
        || lower.contains("picture")
        || lower.contains("generate")
        || lower.contains("movie")
        || lower.contains("tv show")
        || lower.contains("radarr")
        || lower.contains("sonarr")
        || lower.contains("download")
    {
        cats.push(Media);
    }

    // Productivity
    if lower.contains("todo")
        || lower.contains("task")
        || lower.contains("obsidian")
        || lower.contains("note")
        || lower.contains("workspace")
        || lower.contains("reddit")
    {
        cats.push(Productivity);
    }

    // If no specific categories matched (ambiguous message), include all
    if cats.len() <= 2 {
        cats.extend([
            Web,
            Communication,
            Development,
            Scheduling,
            Media,
            Productivity,
        ]);
    }

    cats
}

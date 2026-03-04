use super::cli_types::StatsCommands;
use crate::config::load_config;
use anyhow::Result;

pub(super) fn stats_command(cmd: &StatsCommands) -> Result<()> {
    let config = load_config(None)?;
    let workspace = config.workspace_path();
    let db_path = workspace.join("memory").join("memory.sqlite3");

    if !db_path.exists() {
        anyhow::bail!(
            "memory database not found at {}. Run the agent first to initialize it.",
            db_path.display()
        );
    }

    let db = crate::agent::memory::MemoryDB::new(&db_path)?;

    match cmd {
        StatsCommands::Tokens { days } => {
            let since = (chrono::Utc::now().date_naive()
                - chrono::Duration::days(i64::from(*days)))
            .format("%Y-%m-%d")
            .to_string();
            let summary = db.get_token_summary(&since)?;

            if summary.is_empty() {
                println!("No token usage data in the last {days} days.");
                return Ok(());
            }

            println!(
                "{:<12} {:<30} {:>10} {:>10} {:>6}",
                "Date", "Model", "Input", "Output", "Calls"
            );
            println!("{}", "\u{2500}".repeat(72));

            let mut total_input = 0i64;
            let mut total_output = 0i64;
            let mut total_calls = 0i64;
            for row in &summary {
                println!(
                    "{:<12} {:<30} {:>10} {:>10} {:>6}",
                    row.date,
                    row.model,
                    row.total_input_tokens,
                    row.total_output_tokens,
                    row.call_count,
                );
                total_input += row.total_input_tokens;
                total_output += row.total_output_tokens;
                total_calls += row.call_count;
            }

            println!("{}", "\u{2500}".repeat(72));
            println!(
                "Total: {total_input} input + {total_output} output tokens across {total_calls} calls"
            );
        }
        StatsCommands::Search => {
            let stats = db.get_search_stats()?;
            println!("Memory Search Statistics");
            println!("{}", "\u{2500}".repeat(40));
            println!("Total searches:       {}", stats.total_searches);
            println!("Total hits:           {}", stats.total_hits);
            println!("Avg results/search:   {:.1}", stats.avg_results_per_search);

            let top = db.get_top_sources(10)?;
            if !top.is_empty() {
                println!("\nTop Sources by Hit Count:");
                for (key, count) in &top {
                    println!("  {key:<30} {count} hits");
                }
            }
        }
        StatsCommands::Intent { days } => {
            let since = (chrono::Utc::now().date_naive()
                - chrono::Duration::days(i64::from(*days)))
            .format("%Y-%m-%d")
            .to_string();
            let stats = db.get_intent_stats(&since)?;

            println!("Intent Classification & Hallucination Detection (last {days} days)");
            println!("{}", "\u{2500}".repeat(55));
            println!();
            println!("Classification:");
            println!("  Total classified:     {}", stats.total_classified);
            println!("  Regex action:         {}", stats.regex_action);
            println!("  Semantic action:      {}", stats.semantic_action);
            println!("  Not action:           {}", stats.not_action);
            if stats.avg_semantic_score_action > 0.0 {
                println!(
                    "  Avg semantic score (action):     {:.3}",
                    stats.avg_semantic_score_action
                );
            }
            if stats.avg_semantic_score_non_action > 0.0 {
                println!(
                    "  Avg semantic score (non-action): {:.3}",
                    stats.avg_semantic_score_non_action
                );
            }
            println!();
            println!("Hallucination Detection:");
            println!("  Total caught:         {}", stats.hallucinations_caught);
            println!("  Layer 1 (regex):      {}", stats.layer1_regex);
            println!("  Layer 2 (intent):     {}", stats.layer2_intent);

            let recent = db.get_recent_hallucinations(10)?;
            if !recent.is_empty() {
                println!();
                println!("Recent Hallucinations:");
                for event in &recent {
                    let layer = event.detection_layer.as_deref().unwrap_or("unknown");
                    let preview = event.message_preview.as_deref().unwrap_or_default();
                    println!("  [{}] {}: {:.60}", event.timestamp, layer, preview);
                }
            }
        }
        StatsCommands::Complexity { days } => {
            let since = (chrono::Utc::now().date_naive()
                - chrono::Duration::days(i64::from(*days)))
            .format("%Y-%m-%d")
            .to_string();
            let stats = db.get_complexity_stats(&since)?;

            if stats.total_scored == 0 {
                println!("No complexity routing data in the last {days} days.");
                println!(
                    "Enable complexity routing: add a 'chat' entry to modelRouting.tasks with thresholds and models"
                );
                return Ok(());
            }

            println!("Complexity Routing (last {days} days)");
            println!("{}", "\u{2500}".repeat(55));
            println!("Messages scored:    {}", stats.total_scored);
            println!();

            println!("Tier Distribution:");
            for tier in &stats.tier_counts {
                let pct = (tier.count as f64 / stats.total_scored as f64) * 100.0;
                println!(
                    "  {:<16} {:>4} ({:>5.1}%)  avg score: {:.2}   tokens: {}",
                    format!("{}:", tier.tier),
                    tier.count,
                    pct,
                    tier.avg_score,
                    tier.total_tokens,
                );
            }

            if !stats.force_counts.is_empty() {
                println!();
                println!("Force Overrides:");
                for f in &stats.force_counts {
                    println!("  {:<24} {}", format!("{}:", f.reason), f.count);
                }
            }

            let recent = db.get_recent_complexity_events("heavy", 5)?;
            if !recent.is_empty() {
                println!();
                println!("Recent Heavy Routing:");
                for event in &recent {
                    let model = event.resolved_model.as_deref().unwrap_or("unknown");
                    let preview = event.message_preview.as_deref().unwrap_or_default();
                    let forced_tag = event
                        .forced
                        .as_ref()
                        .map(|f| format!(" [forced:{f}]"))
                        .unwrap_or_default();
                    println!(
                        "  [{}] score={:.2} model={}{} \"{:.60}\"",
                        event.timestamp, event.composite_score, model, forced_tag, preview
                    );
                }
            }
        }
    }

    Ok(())
}

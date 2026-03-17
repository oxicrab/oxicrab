#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use oxicrab::router::context::{ActionDirective, DirectiveTrigger, RouterContext};
use oxicrab::router::MessageRouter;
use serde_json::json;

#[derive(Arbitrary, Debug)]
struct Input {
    message: String,
    active_tool: Option<String>,
    directives: Vec<String>,
    semantic_tools: Vec<String>,
}

fuzz_target!(|input: Input| {
    let router = MessageRouter::new(vec![], vec![], "!".to_string());
    let mut ctx = RouterContext::default();
    if let Some(tool) = input.active_tool {
        ctx.set_active_tool(Some(tool));
    }

    let now = oxicrab::router::now_ms();
    let directives: Vec<ActionDirective> = input
        .directives
        .into_iter()
        .take(8)
        .map(|trigger| ActionDirective {
            trigger: DirectiveTrigger::Exact(trigger),
            tool: "rss".to_string(),
            params: json!({"action":"next"}),
            single_use: true,
            ttl_ms: 30_000,
            created_at_ms: now,
        })
        .collect();
    ctx.install_directives(directives);

    let semantic = if input.semantic_tools.is_empty() {
        None
    } else {
        Some(input.semantic_tools)
    };

    let _ = router.route_with_semantic(&input.message, &ctx, None, semantic);
});

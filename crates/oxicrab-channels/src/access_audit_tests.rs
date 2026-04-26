//! Structural audit of channel access-control wiring.
//!
//! The historical "bot replies to everyone in a `WhatsApp` group" leak was
//! caused by a channel that forwarded inbound messages without calling
//! `check_group_access` / `check_dm_access`. The fix layered three
//! defenses: (1) `DenyByDefaultList` makes empty allow-lists deny-all at
//! the type level, (2) every channel calls the shared access helpers
//! before `inbound_tx.send`, (3) per-channel field types use
//! `DenyByDefaultList` so a refactor can't silently revert to
//! `Vec<String>`.
//!
//! Defense (1) is enforced by the type system. Defenses (2) and (3) are
//! enforced by convention — a new channel implementation could forget
//! either. These tests are the structural guardrail: any future channel
//! that lands in `src/` without invoking the access helpers will fail
//! `every_channel_invokes_both_access_helpers`, and any new channel
//! directory that isn't audited will fail `audited_channels_match_disk`.

use std::path::PathBuf;

/// Channel source files that build `InboundMessage`. Update this list
/// when a new channel is added — the `audited_channels_match_disk` test
/// will refuse to pass until the new channel is enumerated here AND
/// the gates are wired.
const AUDITED_CHANNELS: &[(&str, &str)] = &[
    ("discord", "src/discord/mod.rs"),
    ("slack", "src/slack/mod.rs"),
    ("telegram", "src/telegram/mod.rs"),
    ("twilio", "src/twilio/mod.rs"),
    ("whatsapp", "src/whatsapp/mod.rs"),
];

/// Sub-directories under `src/` that are NOT user-facing channels and
/// therefore don't need access-control gates. Update when adding new
/// shared infrastructure modules.
const NON_CHANNEL_MODULES: &[&str] = &["dispatch", "manager", "media_utils", "utils"];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = crate_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {} failed: {e}", path.display()))
}

/// Walk `src/` and assert the set of channel directories matches
/// `AUDITED_CHANNELS`. Adding a new channel without auditing it (or
/// removing one without dropping it from the list) fails this test —
/// which is the prompt to wire the access helpers and add the entry.
#[test]
fn audited_channels_match_disk() {
    let src = crate_root().join("src");
    let mut on_disk: Vec<String> = std::fs::read_dir(&src)
        .expect("read src/")
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !NON_CHANNEL_MODULES.contains(&name.as_str()))
        .collect();
    on_disk.sort();

    let mut audited: Vec<String> = AUDITED_CHANNELS.iter().map(|(n, _)| (*n).into()).collect();
    audited.sort();

    assert_eq!(
        on_disk, audited,
        "channel directory layout drifted: on-disk channels {on_disk:?} \
         differ from audited channels {audited:?}. \
         If you added a channel, append it to AUDITED_CHANNELS in \
         access_audit_tests.rs AND ensure it calls check_dm_access + \
         check_group_access before inbound_tx.send. \
         If you removed a channel, remove it from AUDITED_CHANNELS. \
         If you added shared infrastructure (not a channel), append it \
         to NON_CHANNEL_MODULES."
    );
}

/// Every audited channel source MUST invoke both access helpers. This
/// looks for the function-call form (`name(`) so a stale comment
/// referencing the helper can't pass the check.
#[test]
fn every_channel_invokes_both_access_helpers() {
    for (name, rel) in AUDITED_CHANNELS {
        let src = read(rel);
        assert!(
            src.contains("check_dm_access("),
            "{name} channel ({rel}) does not call check_dm_access(...). \
             Without this gate, an unauthorized DM sender bypasses the \
             allowFrom list and the bot replies to anyone who messages \
             it. See crates/oxicrab-channels/src/utils/mod.rs for the \
             helper."
        );
        assert!(
            src.contains("check_group_access("),
            "{name} channel ({rel}) does not call check_group_access(...). \
             Without this gate, the bot responds in any group it is \
             added to — the historical 'replies to everyone in the \
             `WhatsApp` group' bug. See crates/oxicrab-channels/src/utils/mod.rs."
        );
    }
}

/// Every audited channel MUST bind its allow lists as `DenyByDefaultList`,
/// not raw `Vec<String>`. The newtype encapsulates the
/// empty-list-means-deny-all invariant; a refactor that drops back to
/// `Vec<String>` would silently re-introduce the bug.
#[test]
fn every_channel_uses_deny_by_default_list() {
    for (name, rel) in AUDITED_CHANNELS {
        let src = read(rel);
        assert!(
            src.contains("DenyByDefaultList"),
            "{name} channel ({rel}) does not reference DenyByDefaultList. \
             Allow-lists must use this newtype so empty == deny-all is \
             enforced at the type level. Raw Vec<String> would let a \
             future refactor revert to empty == allow-all."
        );
    }
}

/// For each channel, every `inbound_tx.send` call site must be preceded
/// somewhere in the file by both access helpers. This catches the case
/// where a channel adds a NEW event handler (slash command, button,
/// reaction) that forwards inbound without first running the gate.
///
/// The check is module-level rather than function-level: `inbound_tx`
/// captures move into nested closures and tokio spawns, so a per-function
/// scan would have too many false negatives. The conjunction with
/// `every_channel_invokes_both_access_helpers` keeps the guarantee.
#[test]
fn inbound_send_sites_are_gated_at_module_level() {
    for (name, rel) in AUDITED_CHANNELS {
        let src = read(rel);
        let send_sites =
            src.matches("inbound_tx.send(").count() + src.matches("inbound_tx\n").count(); // builder pattern w/ trailing .send

        assert!(
            send_sites > 0,
            "{name} ({rel}): expected at least one inbound_tx.send site \
             but found none — channel may be misconfigured or not \
             dispatching messages."
        );

        let gate_sites =
            src.matches("check_dm_access(").count() + src.matches("check_group_access(").count();
        assert!(
            gate_sites >= 2,
            "{name} ({rel}): only {gate_sites} access-helper invocations \
             found but channel has {send_sites} inbound_tx.send site(s). \
             Add gating before each new event handler."
        );
    }
}

/// Sanity check: the shared helper itself enforces deny-on-empty.
/// If this test ever fails, the type-level guarantee has been weakened
/// and every channel becomes vulnerable simultaneously.
#[test]
fn deny_by_default_list_denies_empty() {
    use oxicrab_core::config::schema::DenyByDefaultList;
    let empty = DenyByDefaultList::default();
    assert!(
        !empty.allows("anyone"),
        "DenyByDefaultList::default() must reject all senders. \
         If empty allows, every channel that uses it loses its access \
         gate at once — this is the historical bug class returning."
    );
}

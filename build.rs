use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=CARGO_BUILD_TARGET");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=CI_COMMIT_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_VERSION");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    let profile = build_profile().unwrap_or_else(|| "unknown".to_string());
    let target = build_target().unwrap_or_else(|| "unknown".to_string());
    let git_sha = git_sha().unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=OXICRAB_BUILD_PROFILE={profile}");
    println!("cargo:rustc-env=OXICRAB_BUILD_TARGET={target}");
    println!("cargo:rustc-env=OXICRAB_GIT_SHA={git_sha}");
}

fn build_profile() -> Option<String> {
    env::var("PROFILE")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(
            || match (env::var("DEBUG").ok(), env::var("OPT_LEVEL").ok()) {
                (Some(debug), _) if debug == "true" => Some("debug".to_string()),
                (_, Some(opt)) if opt == "0" => Some("debug".to_string()),
                (_, Some(_)) => Some("release".to_string()),
                _ => None,
            },
        )
}

fn build_target() -> Option<String> {
    env::var("TARGET")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            env::var("CARGO_BUILD_TARGET")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            let arch = env::var("CARGO_CFG_TARGET_ARCH").ok()?;
            let os = env::var("CARGO_CFG_TARGET_OS").ok()?;
            let env_abi = env::var("CARGO_CFG_TARGET_ENV").ok();
            if let Some(env_abi) = env_abi
                && !env_abi.is_empty()
            {
                return Some(format!("{arch}-unknown-{os}-{env_abi}"));
            }
            Some(format!("{arch}-unknown-{os}"))
        })
        .or_else(|| env::var("HOST").ok().filter(|v| !v.is_empty()))
}

fn git_sha() -> Option<String> {
    for key in [
        "OXICRAB_GIT_SHA",
        "GITHUB_SHA",
        "CI_COMMIT_SHA",
        "SOURCE_VERSION",
    ] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.chars().take(12).collect());
            }
        }
    }

    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim().to_string();
    if sha.is_empty() { None } else { Some(sha) }
}

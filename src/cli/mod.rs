// CLI subcommands are operator-facing; print macros are how they
// communicate. Workspace lints warn on `print_stdout` / `print_stderr`;
// this allow makes the CLI tree exempt without per-fn clutter.
#![allow(clippy::print_stdout, clippy::print_stderr)]

pub mod commands;
pub mod doctor;

pub use commands::run;

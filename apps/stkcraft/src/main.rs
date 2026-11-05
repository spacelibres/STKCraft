//! `STKCraft`: space mission analysis and visualisation.
//!
//! The binary is a thin transport over `stkcraft-engine`. Every subcommand maps to a command
//! id in the engine's registry, so what the CLI can do, a menu and an agent can do too — the
//! CLI is not a separate implementation.
//!
//! ```text
//! stkcraft --help
//! stkcraft passes --tle "1 25544U ..." "2 25544 ..." --lat 51.5 --lon -0.13
//! ```

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

mod cli;
mod render;

use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run(std::env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

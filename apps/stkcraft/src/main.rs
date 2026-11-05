//! `STKCraft`: space mission analysis and visualisation.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

fn main() {
    println!("STKCraft {}", env!("CARGO_PKG_VERSION"));
}

//! The `STKCraft` engine: the scenario, the commands, and the analysis that runs over them.
//!
//! `STKCraft` is engine-first. Everything a user can do is a **command** with an id, a label, a
//! menu path, parameter documentation, an `enabled` predicate and a `run` function. The menus,
//! shortcuts, CLI, control channel and MCP server will all dispatch by id, which is what lets
//! an agent do anything a user can — and it is why the engine comes before the viewport.
//!
//! The layers below supply the physics:
//!
//! - `stkcraft-framework-time` — epochs, scales and leap seconds.
//! - `stkcraft-astro-sgp4` — a two-line element set to a TEME state.
//! - `stkcraft-astro-frames` — TEME to GCRF and to Earth-fixed.
//! - `stkcraft-astro-access` — a station, a sensor and a position to access windows.
//!
//! This crate holds them together: it owns the scenario document, moves each satellite from
//! its element set through the frame chain to an Earth-fixed position, and hands that to the
//! access layer.
//!
//! # Not yet built
//!
//! There is no undo. The command table is exercised through the same dispatch path the window,
//! CLI, control channel and MCP server use; those transports live in their own crates and reach
//! the engine only through the registry.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod analysis;
pub mod central;
pub mod commands;
pub mod planet_access;
pub mod propagate;
pub mod scenario;

pub use analysis::{SatelliteAccess, compute_access, compute_all_access};
pub use central::PlanetOrbit;
pub use commands::{Arg, Args, Command, CommandOutcome, Parameter, Registry, args_from};
pub use planet_access::{PlanetPass, find_passes};
pub use propagate::Propagator;
pub use scenario::{Satellite, SatelliteId, Scenario, Station, StationId};

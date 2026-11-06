//! The crate layout, and the rule that enforces it.
//!
//! `STKCraft` is engine-first. The analysis code knows nothing about windows, GPU state or
//! the user interface, so it can run in a headless CLI, in a browser tab and inside the
//! desktop app without changes. Layering is what keeps that true as the codebase grows,
//! and it has to be enforced mechanically because a single convenient `use` can undo it.
//!
//! ```text
//! L0  core      units, errors, vectors
//! L1  time      time scales, leap seconds, epochs
//! L2  frames    reference frames and rotations
//! L2  sgp4      orbit propagation
//! L3  access    access and coverage geometry
//! L4  engine    scenario, commands and analysis
//! L5  mcp       the command registry spoken to an outside client
//! L5  viewport  3D rendering (GPU), above the analysis layers
//! L6  agent     a model driving the engine through the MCP tools
//! ...
//!     apps      the desktop app, CLI and web shell
//! ```
//!
//! A crate may depend only on a strictly lower layer. The apps sit at
//! [`FIRST_LAYER`] + [`LAYER_COUNT`] and may use anything.

/// A crate in the workspace and the layer it belongs to.
#[derive(Debug, Clone, Copy)]
pub struct CrateSpec {
    /// The package name as it appears in `Cargo.toml`.
    pub name: &'static str,
    /// The directory holding the crate, relative to the workspace root.
    pub path: &'static str,
    /// The layer number. Lower is more fundamental.
    pub layer: u32,
}

/// The lowest layer number used by any crate.
#[cfg_attr(not(test), allow(dead_code))]
pub const FIRST_LAYER: u32 = 0;

/// How many layers the analysis crates occupy. The apps sit just above the top one.
///
/// Layers 0 to 5 were not enough once the agent landed: the MCP server is a transport onto the
/// engine (L5) and the agent drives the engine *through that server* (L6), so the two need layers
/// of their own. The viewport keeps the transport's height — nothing below the app consumes it.
pub const LAYER_COUNT: u32 = 7;

/// The highest layer that must stay free of any GUI dependency.
///
/// Everything at or below this layer must be usable from a test, a server or a
/// WebAssembly build with no window. Only the UI crates and the apps may pull in a
/// windowing or immediate-mode GUI crate.
pub const LAST_NON_UI_LAYER: u32 = 2;

/// The layer number given to the apps, which sit above every analysis crate.
pub const APP_LAYER: u32 = LAYER_COUNT;

/// The layer number given to the build tool. It is not an analysis crate and depends on
/// nothing else here; it reads the workspace manifests, so it is kept out of the layering
/// graph by being placed above the apps.
pub const TOOL_LAYER: u32 = LAYER_COUNT + 1;

/// Crates that only the UI layer and the apps may depend on.
///
/// The 3D viewport has landed; these are the crates it is allowed to pull in and no analysis
/// crate is. `wgpu` is listed because a renderer is the thing the rule is about, even though the
/// engine never names it.
pub const GUI_CRATES: [&str; 7] = [
    "egui",
    "eframe",
    "egui-winit",
    "egui-wgpu",
    "winit",
    "rfd",
    "wgpu",
];

/// Every analysis crate in the workspace, in dependency order.
///
/// Register a new crate here in the same change that adds it: `cargo xtask layers`
/// fails for a workspace member that is missing from this table.
pub const LAYOUT: &[CrateSpec] = &[
    CrateSpec {
        name: "stkcraft-core",
        path: "crates/core",
        layer: 0,
    },
    // Interface translations have no dependency at all: they are a text format and a lookup. They
    // sit beside `core` because the engine's own messages are user-visible and reach them too.
    CrateSpec {
        name: "stkcraft-i18n",
        path: "crates/i18n",
        layer: 0,
    },
    CrateSpec {
        name: "stkcraft-time",
        path: "crates/time",
        layer: 1,
    },
    // Body constants and rotation models sit beside the frame machinery: both are L2, and
    // neither depends on the other, which is why `stkcraft-body` does not need `Rotation3`
    // from `stkcraft-frames`.
    CrateSpec {
        name: "stkcraft-body",
        path: "crates/body",
        layer: 2,
    },
    CrateSpec {
        name: "stkcraft-frames",
        path: "crates/frames",
        layer: 2,
    },
    CrateSpec {
        name: "stkcraft-sgp4",
        path: "crates/sgp4",
        layer: 2,
    },
    CrateSpec {
        name: "stkcraft-access",
        path: "crates/access",
        layer: 3,
    },
    CrateSpec {
        name: "stkcraft-ephemeris",
        path: "crates/ephemeris",
        layer: 3,
    },
    // The extension package contract: manifests, ids and trust tiers. Pure data and validation, so
    // it sits with the analysis layers even though only the app consumes it.
    CrateSpec {
        name: "stkcraft-extensions",
        path: "crates/extensions",
        layer: 3,
    },
    CrateSpec {
        name: "stkcraft-engine",
        path: "crates/engine",
        layer: 4,
    },
    // The MCP server is a transport onto the engine: a menu, a CLI and an MCP session all reach
    // the same command registry, and nothing below knows any of them exists. It sits with the
    // viewport, one below the agent, because it consumes the engine and nothing consumes it.
    CrateSpec {
        name: "stkcraft-mcp",
        path: "crates/mcp",
        layer: APP_LAYER - 2,
    },
    // The 3D viewport sits above the analysis layers because it is a UI crate: it may use
    // `wgpu` and `egui`, which `LAST_NON_UI_LAYER` forbids below. It still depends only on
    // `stkcraft-core`, so it can be replaced without touching the engine.
    CrateSpec {
        name: "stkcraft-viewport",
        path: "crates/viewport",
        layer: APP_LAYER - 2,
    },
    // The agent drives the engine through the MCP server's tools, so it sits strictly above that
    // server and knows nothing the protocol does not expose. Replacing the model, or talking to the
    // server from another process, leaves this crate out of the picture entirely.
    CrateSpec {
        name: "stkcraft-agent",
        path: "crates/agent",
        layer: APP_LAYER - 1,
    },
    CrateSpec {
        name: "stkcraft",
        path: "apps/stkcraft",
        layer: APP_LAYER,
    },
    CrateSpec {
        name: "xtask",
        path: "xtask",
        layer: TOOL_LAYER,
    },
];

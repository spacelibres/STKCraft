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
//! L4  viewport  3D rendering (GPU), above the analysis layers
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
pub const LAYER_COUNT: u32 = 5;

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
    CrateSpec {
        name: "stkcraft-engine",
        path: "crates/engine",
        layer: 4,
    },
    // The 3D viewport sits above the analysis layers because it is a UI crate: it may use
    // `wgpu` and `egui`, which `LAST_NON_UI_LAYER` forbids below. It still depends only on
    // `stkcraft-core`, so it can be replaced without touching the engine.
    CrateSpec {
        name: "stkcraft-viewport",
        path: "crates/viewport",
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

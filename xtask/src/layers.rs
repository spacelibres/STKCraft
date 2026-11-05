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
pub const LAYER_COUNT: u32 = 4;

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
/// Extend this list when the 3D viewport and the egui shell land.
pub const GUI_CRATES: [&str; 6] = ["egui", "eframe", "egui-winit", "egui-wgpu", "winit", "rfd"];

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

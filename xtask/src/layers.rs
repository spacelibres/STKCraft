//! The crate layout, and the rule that enforces it.
//!
//! `STKCraft` is engine-first. The analysis code knows nothing about windows, GPU state or
//! the user interface, so it can run in a headless CLI, in a browser tab and inside the
//! desktop app without changes. Layering is what keeps that true as the codebase grows,
//! and it has to be enforced mechanically because a single convenient `use` can undo it.
//!
//! ```text
//! L0  stkcraft-core               units, errors, vectors
//! L0  stkcraft-i18n               interface translations
//! L1  stkcraft-time               time scales, leap seconds, epochs
//! L1  stkcraft-storage            the record, the cache and the vector store
//! L2  stkcraft-astro-body         body constants and rotation models
//! L2  stkcraft-astro-frames       reference frames and rotations
//! L2  stkcraft-astro-sgp4         orbit propagation
//! L3  stkcraft-astro-access       access and coverage geometry
//! L3  stkcraft-astro-ephemeris    SPICE kernels and analytic ephemerides
//! L3  stkcraft-agent-knowledge    the user's documents, and the search over them
//! L3  stkcraft-extensions         the extension package contract
//! L4  stkcraft-astro-engine       scenario, commands and analysis
//! L5  stkcraft-agent-mcp          the command registry spoken to an outside client
//! L5  stkcraft-ui-viewport        3D rendering (GPU), above the analysis layers
//! L6  stkcraft-agent-runtime      a model driving the engine through the MCP tools
//! L6  stkcraft-agent-control      a launcher driving the same registry
//! ...
//!     stkcraft                    the desktop app, CLI and web shell
//! ```
//!
//! The name of a crate states what it is, and the directory above it states the same thing: a
//! package is `stkcraft-<group>-<leaf>` when it belongs to a group (`astro`, `agent`, `ui`) and
//! bare `stkcraft-<subject>` only for the platform, which is the part a sibling app shares rather
//! than the part that knows what an orbit is. The table below is the check: a name that disagrees
//! with its directory, or a directory that disagrees with its layer, fails the gate.
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

/// The package name a crate's directory implies, or `None` for a path the rule does not cover.
///
/// `crates/<group>/<leaf>` is `stkcraft-<group>-<leaf>`, except under `crates/framework/`, which is
/// the platform — what a sibling app shares, rather than the part that knows what an orbit is — and
/// keeps the bare `stkcraft-<leaf>`. The apps and the build tool sit outside `crates/` and are named
/// by their own rule, so they have no implied name. Rules: `docs/standards.md` §7.
pub fn implied_name(path: &str) -> Option<String> {
    let mut segments = path.split('/');
    if segments.next()? != "crates" {
        return None;
    }
    let group = segments.next()?;
    let leaf = segments.next()?;
    if segments.next().is_some() || leaf.is_empty() {
        return None;
    }
    Some(if group == "framework" {
        format!("stkcraft-{leaf}")
    } else {
        format!("stkcraft-{group}-{leaf}")
    })
}

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
        path: "crates/framework/core",
        layer: 0,
    },
    // Interface translations have no dependency at all: they are a text format and a lookup. They
    // sit beside `core` because the engine's own messages are user-visible and reach them too.
    CrateSpec {
        name: "stkcraft-i18n",
        path: "crates/framework/i18n",
        layer: 0,
    },
    // Persistence is infrastructure: the record, the cache and the vector store depend on `core` and
    // nothing else, and every layer above reads through them (docs/design/storage.md).
    CrateSpec {
        name: "stkcraft-storage",
        path: "crates/framework/storage",
        layer: 1,
    },
    CrateSpec {
        name: "stkcraft-time",
        path: "crates/framework/time",
        layer: 1,
    },
    // Body constants and rotation models sit beside the frame machinery: both are L2, and
    // neither depends on the other, which is why `stkcraft-astro-body` does not need `Rotation3`
    // from `stkcraft-astro-frames`.
    CrateSpec {
        name: "stkcraft-astro-body",
        path: "crates/astro/body",
        layer: 2,
    },
    CrateSpec {
        name: "stkcraft-astro-frames",
        path: "crates/astro/frames",
        layer: 2,
    },
    CrateSpec {
        name: "stkcraft-astro-sgp4",
        path: "crates/astro/sgp4",
        layer: 2,
    },
    CrateSpec {
        name: "stkcraft-astro-access",
        path: "crates/astro/access",
        layer: 3,
    },
    CrateSpec {
        name: "stkcraft-astro-ephemeris",
        path: "crates/astro/ephemeris",
        layer: 3,
    },
    // The user's own documents and the search over them. Pure text processing: no network, no
    // model, so it sits with the analysis layers and can be tested without either. The crate name
    // says `agent` because a model is what reads the answers, and the layer says 3 because the
    // engine's knowledge commands call the same search with no model in the picture.
    CrateSpec {
        name: "stkcraft-agent-knowledge",
        path: "crates/agent/knowledge",
        layer: 3,
    },
    // The extension package contract: manifests, ids and trust tiers. Pure data and validation, so
    // it sits with the analysis layers even though only the app consumes it.
    CrateSpec {
        name: "stkcraft-extensions",
        path: "crates/framework/extensions",
        layer: 3,
    },
    CrateSpec {
        name: "stkcraft-astro-engine",
        path: "crates/astro/engine",
        layer: 4,
    },
    // The MCP server is a transport onto the engine: a menu, a CLI and an MCP session all reach
    // the same command registry, and nothing below knows any of them exists. It sits with the
    // viewport, one below the agent, because it consumes the engine and nothing consumes it.
    CrateSpec {
        name: "stkcraft-agent-mcp",
        path: "crates/agent/mcp",
        layer: APP_LAYER - 2,
    },
    // The 3D viewport sits above the analysis layers because it is a UI crate: it may use
    // `wgpu` and `egui`, which `LAST_NON_UI_LAYER` forbids below. It still depends only on
    // `stkcraft-core`, so it can be replaced without touching the engine.
    CrateSpec {
        name: "stkcraft-ui-viewport",
        path: "crates/ui/viewport",
        layer: APP_LAYER - 2,
    },
    // The agent drives the engine through the MCP server's tools, so it sits strictly above that
    // server and knows nothing the protocol does not expose. Replacing the model, or talking to the
    // server from another process, leaves this crate out of the picture entirely.
    CrateSpec {
        name: "stkcraft-agent-runtime",
        path: "crates/agent/runtime",
        layer: APP_LAYER - 1,
    },
    // The control channel is the same relationship for a launcher instead of a model: it drives the
    // registry through the MCP server, so it sits at the agent's height rather than the server's.
    CrateSpec {
        name: "stkcraft-agent-control",
        path: "crates/agent/control",
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

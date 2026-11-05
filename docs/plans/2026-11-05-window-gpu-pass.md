# The window's `wgpu` pass

**Goal:** the window draws through the viewport's `wgpu` pipelines instead of `egui`'s painter, so
the Earth is a lit globe, the star field is drawn, and a ground track on the far side is hidden by
the Earth rather than painted over it.

**Starting point:** 0.1.0 shipped a window that projected vertices on the CPU and handed the
painter line segments, while the offscreen verification path used the real pipelines. Both consumed
the same `SceneData`, so the geometry agreed, but the window had no depth buffer and no globe.

## The constraint that decides the design

`egui` owns the surface and records one render pass per frame. A `wgpu` renderer cannot open a
second pass into the same surface, so there are three ways in:

1. **Draw into `egui`'s pass with a paint callback** (`egui_wgpu::CallbackTrait`). Needs the pass
   to have a depth attachment, which `eframe` creates only when
   `NativeOptions::depth_buffer` is non-zero.
2. Render to an offscreen texture with its own depth buffer and blit the result as an `egui`
   image. Always works, costs a second pass, a full-screen blit and a texture per resize.
3. A second swapchain. Not portable, and it fights `eframe` over presentation.

Option 1 is the answer: `eframe` 0.37 takes `depth_buffer: u8`, resolves it with
`egui_wgpu::depth_format_from_bits`, and attaches the depth texture to the pass with
`LoadOp::Clear(1.0)`. `egui`'s own pipeline uses `depth_compare: Always` and writes no depth, so
it neither disturbs our depth buffer nor needs one.

The coupling that could break silently is bits-versus-format: `eframe` is configured in bits and a
`wgpu` pipeline in a `TextureFormat`. `renderer::WINDOW_DEPTH_BITS` is the single number, the app
reads it for `NativeOptions`, and a test asserts
`depth_format_from_bits(WINDOW_DEPTH_BITS, 0) == Some(DEPTH_FORMAT)`.

## Design decisions

**Pipelines live in the viewport crate, not in the app.** `SceneRenderer` owns the two pipelines,
the uniform, and one growable buffer per drawable. The verification path previously created the
same descriptors inline; two copies would have been two things to keep in step, and the whole claim
of the screenshot path is that it draws what the window draws. Buffers are written with
`write_buffer` and reallocated only when the data outgrows them, instead of a fresh
`create_buffer_init` per drawable per frame.

**`SceneCallback` carries geometry, not a device.** `prepare` fills the buffers; `paint` records
the draws into the pass `egui` is already recording. The renderer lives in
`callback_resources`, so it survives across frames, and it is rebuilt if the surface format changes
underneath a running window (a monitor on another GPU).

**The aspect ratio comes from the callback's rectangle in points.** The ratio is the same in points
and pixels, and the rectangle is known before the callback is created — the pixels-per-point scale
is not. A degenerate or non-finite ratio becomes `1.0` rather than a singular projection.

**The panel draws no frame.** `egui`'s default central-panel margin put a theme-coloured band
around the viewport, which reads as a letterbox. `Frame::NONE` makes the viewport the whole area —
and, usefully, makes the window's path and the offscreen path cover the same pixels.

## Verification, because a window cannot be opened here

The machine has no window system, so "the window draws it" cannot be checked by opening a window.
Two offscreen paths, both keeping the depth buffer they need:

- `STKCRAFT_SCREENSHOT` — the viewport's own pass, as before.
- `STKCRAFT_SCREENSHOT_EGUI` — a full `egui` frame through `run_ui` (the entry point `eframe`
  itself calls), tessellated, and rendered with `egui_wgpu::Renderer` at
  `depth_stencil_format: Some(DEPTH_FORMAT)`. This is the window's code path minus the surface.

Evidence gathered, in order:

1. The refactored direct path is **byte-identical** to the committed 0.1.0 reference image
   (`docs/images/viewport-0.1.0.png`), so the move into `SceneRenderer` changed no pixels.
2. The `egui` path's image is **byte-identical** to the direct path's, so the window's callback,
   projection, viewport mapping and depth attachment all behave the same way.
3. A numeric occlusion check runs inside both paths on every verification render: every track
   vertex behind the globe must show no track colour at its projected pixel, and the vertices in
   front are counted as the control that the check can see a line at all. The default frame gives
   `100 far-side samples checked, 0 drawn through the globe; control: 128/128 near-side drawn`.
4. **The check was falsified before it was trusted**: with the line pipeline's `depth_compare`
   changed to `Always` — the painter's behaviour — it reports 100 of 100 far-side samples drawn and
   exits non-zero. It fails for the defect it exists to catch, and only for that reason.

## Not done here

- **Thick lines.** `wgpu` has no portable line width, so a wide line needs screen-space quad
  expansion: two triangles per segment, expanded in the vertex shader by half the width along the
  screen-space normal. That is the next arc, and it is a geometry change in `geometry.rs`, not a
  pipeline parameter.
- **MSAA.** `eframe` supports it, but every pipeline must be built for the sample count and the
  format checked against the adapter, so it is a decision with a cost (memory, bandwidth) rather
  than a flag.

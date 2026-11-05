//! Rendering one frame offscreen, for verification.
//!
//! The window path cannot be inspected from a build machine with no window system, so this module
//! draws the app's frame into a texture and writes a PNG. It is how the claim "the viewport works"
//! is checked rather than asserted: the pipeline descriptors, the WGSL compilation, the depth
//! buffer and the geometry all get exercised, and the result is an image that can be looked at.
//!
//! There are two entry points, because the window has two layers and either can be the broken one:
//!
//! * [`screenshot`] opens its own render pass and draws the scene directly. It answers "are the
//!   pipelines and the geometry right?".
//! * [`screenshot_through_egui`] builds an `egui` frame the way the window does, with the scene as
//!   a paint callback, and renders it with `egui-wgpu`'s renderer and a depth attachment. It
//!   answers "does the window's own path draw that scene?", which the direct path cannot: a depth
//!   attachment `eframe` never created, a projection built from the wrong aspect ratio, or a
//!   callback that never runs would all pass the direct path and still leave the window blank.
//!
//! Both are switched on by an environment variable (`STKCRAFT_SCREENSHOT` and
//! `STKCRAFT_SCREENSHOT_EGUI`) and are otherwise never reachable: a normal run opens a window.

use egui::Sense;
use stkcraft_viewport::renderer::{DEPTH_FORMAT, Globals, SceneRenderer};
use stkcraft_viewport::scene::SceneData;
use stkcraft_viewport::{Camera, SceneCallback};

use crate::app::App;

/// The texture format used for the offscreen target.
///
/// `Rgba8Unorm` and not `Rgba8UnormSrgb`: the viewport's colours are already linear and the
/// transfer function is applied when the PNG is written, so the format must not apply it too.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The size of the verification image, in pixels.
pub const WIDTH: u32 = 960;
/// See [`WIDTH`].
pub const HEIGHT: u32 = 600;

/// Space, as the clear colour of a pass.
///
/// Linear RGB, the same colour the window paints behind the scene.
const SPACE: wgpu::Color = wgpu::Color {
    r: 0.016,
    g: 0.024,
    b: 0.047,
    a: 1.0,
};

/// An offscreen adapter, the device created from it and its queue.
///
/// Kept together so both verification paths open the GPU the same way, and so the adapter is
/// always printed: a software Vulkan fallback must be visible in the output rather than silently
/// behind a claim about how the viewport renders.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    /// Open the best available offscreen adapter.
    ///
    /// # Errors
    ///
    /// The message says which step failed — no adapter, or no device — because a build machine
    /// without a driver is a normal condition that needs a diagnosis, not a panic.
    fn open() -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            // No surface: this is deliberately an offscreen adapter, so it can be the software
            // Vulkan implementation on a machine with no display.
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|error| format!("no GPU adapter: {error}"))?;

        let info = adapter.get_info();
        eprintln!(
            "adapter: {} ({:?}, {:?})",
            info.name, info.backend, info.device_type
        );

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("stkcraft screenshot"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        }))
        .map_err(|error| format!("could not create a device: {error}"))?;

        Ok(Self { device, queue })
    }

    /// A colour target and a depth target of the verification size.
    fn targets(&self) -> (wgpu::Texture, wgpu::Texture) {
        let size = wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        };
        let colour = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("colour"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        (colour, depth)
    }

    /// Read a colour target back as 8-bit RGB.
    ///
    /// # Errors
    ///
    /// A message if the GPU never finishes or the mapping fails; both are reported rather than
    /// skipped, because a partially read frame would be a misleading image.
    fn read_rgb(&self, texture: &wgpu::Texture) -> Result<Vec<u8>, String> {
        // Reading a texture back needs bytes per row aligned to 256.
        let unpadded_row = WIDTH * 4;
        let padded_row = unpadded_row.div_ceil(256) * 256;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded_row) * u64::from(HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(HEIGHT),
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|error| format!("the GPU did not finish: {error}"))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|error| format!("could not read the frame back: {error}"))?;
        let mut rgb = Vec::with_capacity((WIDTH * HEIGHT * 3) as usize);
        for row in 0..HEIGHT {
            let start = (row * padded_row) as usize;
            let end = start + unpadded_row as usize;
            let Some(line) = mapped.get(start..end) else {
                continue;
            };
            let (pixels, _) = line.as_chunks::<4>();
            for [r, g, b, _] in pixels {
                rgb.extend_from_slice(&[*r, *g, *b]);
            }
        }
        drop(mapped);
        readback.unmap();
        Ok(rgb)
    }
}

/// Write an 8-bit RGB image to `path`.
///
/// # Errors
///
/// A message for the user if the file cannot be created or written.
fn write_rgb(rgb: &[u8], path: &str) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|error| format!("{path}: {error}"))?;
    write_png(&mut file, WIDTH, HEIGHT, rgb).map_err(|error| format!("{path}: {error}"))
}

/// Render the app's current frame to a PNG at `path`, through the viewport's own render pass.
///
/// # Errors
///
/// A message for the user: no adapter, no device, a shader that will not assemble, or a file that
/// cannot be written. Every distinct failure says which one it was, because "the screenshot did
/// not work" is not actionable.
pub fn screenshot(app: &App, path: &str) -> Result<(), String> {
    let (epoch_label, scene, camera) = app.scene_for_verification();
    let globals = Globals::new(
        camera.view(),
        camera.projection(WIDTH as f32 / HEIGHT as f32),
    );

    let gpu = Gpu::open()?;

    // The pipelines, the shader and the buffers are the viewport crate's, not this module's: this
    // path exists to draw the scene the window draws, so it must not own a second copy of it.
    let mut renderer = SceneRenderer::new(&gpu.device, TARGET_FORMAT, DEPTH_FORMAT);
    renderer.upload(&gpu.device, &gpu.queue, &globals, &scene);

    let (color_texture, depth_texture) = gpu.targets();
    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

    eprintln!(
        "camera: {:.0} km from the centre, near {:.0} km, far {:.0} km, {} stars",
        camera.distance_km,
        camera.near_plane_km(),
        camera.far_plane_km(),
        scene.stars.len() / 2
    );

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        });
    {
        // `forget_lifetime` because `SceneRenderer::draw` takes a `RenderPass<'static>` — the same
        // shape `egui-wgpu` hands a paint callback. The pass is dropped at the end of this block,
        // which ends it before the encoder is used for the readback copy.
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SPACE),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        // The depth buffer is not read after the frame, so it is not kept.
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            })
            .forget_lifetime();
        renderer.draw(&mut pass);
    }
    gpu.queue.submit([encoder.finish()]);

    let rgb = gpu.read_rgb(&color_texture)?;
    assert_occlusion(&scene, &camera, &rgb)?;
    write_rgb(&rgb, path)?;
    eprintln!(
        "scene at {}: {} track vertices, {} markers",
        epoch_label,
        scene.tracks.len(),
        scene.markers.len()
    );
    Ok(())
}

/// Render the app's current frame to a PNG the way the window renders it: as an `egui` paint
/// callback inside `egui-wgpu`'s own render pass, with a depth attachment.
///
/// Nothing here is specific to this module except the frame's plumbing — the callback, the
/// pipelines and the geometry all come from the viewport crate, which is the point: what is
/// exercised is what the window runs.
///
/// # Errors
///
/// A message for the user: no adapter, no device, or a file that cannot be written.
pub fn screenshot_through_egui(app: &App, path: &str) -> Result<(), String> {
    let (epoch_label, scene, camera) = app.scene_for_verification();
    let gpu = Gpu::open()?;

    // One `egui` frame whose whole area is the viewport, with the scene as a paint callback over
    // it. `run_ui` is the entry point `eframe` itself uses, so the layout path is the window's.
    let context = egui::Context::default();
    let screen = egui::vec2(WIDTH as f32, HEIGHT as f32);
    let raw_input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
        ..Default::default()
    };
    let mut callback = Some(SceneCallback::new(
        scene.clone(),
        &camera,
        WIDTH as f32 / HEIGHT as f32,
        TARGET_FORMAT,
    ));
    let output = context.run_ui(raw_input, |ui| {
        // The window's own frame: no panel margin, so the callback covers the whole image and the
        // occlusion check's screen coordinates are the image's coordinates.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
                let rect = response.rect;
                // The same background the window paints, underneath the scene.
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(4, 6, 12));
                if let Some(callback) = callback.take() {
                    painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect, callback,
                    ));
                }
            });
    });
    let pixels_per_point = output.pixels_per_point;
    let primitives = context.tessellate(output.shapes, pixels_per_point);

    let mut renderer = eframe::egui_wgpu::Renderer::new(
        &gpu.device,
        TARGET_FORMAT,
        eframe::egui_wgpu::RendererOptions {
            // The window's depth buffer, which is what lets the far side of the Earth hide a
            // ground track behind it. One sample, because the viewport's pipelines are built for
            // one.
            depth_stencil_format: Some(DEPTH_FORMAT),
            msaa_samples: 1,
            // Dithering would add noise to the image for no benefit on a verification frame, and
            // it would make two renders of one frame differ in the low bits.
            dithering: false,
            predictable_texture_filtering: true,
        },
    );
    // `egui` 0.37 hands several deltas per texture in one entry; each is an upload of its own.
    for (id, deltas) in &output.textures_delta.set {
        for delta in deltas {
            renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }

    let (color_texture, depth_texture) = gpu.targets();
    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let screen_descriptor = eframe::egui_wgpu::ScreenDescriptor {
        size_in_pixels: [WIDTH, HEIGHT],
        pixels_per_point,
    };

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui frame"),
        });
    let commands = renderer.update_buffers(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &primitives,
        &screen_descriptor,
    );
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The window's surface is cleared before anything paints; space is the
                        // colour it ends up with everywhere the scene does not cover.
                        load: wgpu::LoadOp::Clear(SPACE),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            })
            .forget_lifetime();
        renderer.render(&mut pass, &primitives, &screen_descriptor);
    }
    gpu.queue.submit(
        commands
            .into_iter()
            .chain(std::iter::once(encoder.finish())),
    );

    let rgb = gpu.read_rgb(&color_texture)?;
    assert_occlusion(&scene, &camera, &rgb)?;
    write_rgb(&rgb, path)?;
    eprintln!(
        "scene at {}: {} track vertices, {} markers, through egui",
        epoch_label,
        scene.tracks.len(),
        scene.markers.len()
    );
    Ok(())
}

/// What one occlusion check found.
///
/// `far_*` counts ground-track samples that are geometrically behind the globe, so depth testing
/// must hide them; `near_*` counts samples in front of it, which must be visible. The near-side
/// count is the control: it proves the sampler can see a line at all, so a zero far-side violation
/// count cannot be an artefact of sampling the wrong pixels.
#[derive(Debug, Default)]
struct Occlusion {
    /// Far-side samples that were far enough inside the disc and clear of the visible track to be
    /// unambiguous.
    far_samples: usize,
    /// Far-side samples whose pixel showed the track: the depth test did not hide them.
    far_violations: usize,
    /// Near-side samples that projected onto the image.
    near_samples: usize,
    /// Near-side samples whose pixel showed the track.
    near_drawn: usize,
}

/// Check that the depth buffer hid the far side of every ground track, and that it did not hide
/// the near side.
///
/// This is the pixel-level version of the claim the window path made and could not keep before it
/// used a depth buffer. It is quantitative rather than visual so it can run on a machine with no
/// display, which is where this project's verification happens.
fn occlusion(scene: &SceneData, camera: &Camera, rgb: &[u8]) -> Occlusion {
    let radius = globe_radius_km(scene);
    let eye = f64_3(camera.eye());
    let clip = stkcraft_viewport::camera::multiply(
        camera.projection(WIDTH as f32 / HEIGHT as f32),
        camera.view(),
    );

    // Project every track vertex once, and decide which side of the globe it belongs to.
    let mut near: Vec<(f64, f64, [u8; 3])> = Vec::new();
    let mut far: Vec<(f64, f64, [u8; 3])> = Vec::new();
    for vertex in &scene.tracks {
        let point = f64_3(vertex.position);
        let Some(hidden) = hidden_by_globe(eye, point, radius) else {
            // Grazing the limb: which side it is on is not a question with a stable answer.
            continue;
        };
        let Some(ndc) = stkcraft_viewport::camera::transform_point(clip, vertex.position) else {
            continue;
        };
        if !(ndc[0].is_finite() && ndc[1].is_finite() && ndc[2].is_finite()) {
            continue;
        }
        // Outside the image, or outside the projection's depth range: not drawn either way.
        if ndc[0].abs() > 1.0 || ndc[1].abs() > 1.0 || !(0.0..=1.0).contains(&ndc[2]) {
            continue;
        }
        let x = f64::from(ndc[0] * 0.5 + 0.5) * f64::from(WIDTH - 1);
        // `Normalized Device Coordinates` have y up; an image has y down.
        let y = (1.0 - f64::from(ndc[1] * 0.5 + 0.5)) * f64::from(HEIGHT - 1);
        let colour = linear_to_bytes(vertex.color);
        if hidden {
            far.push((x, y, colour));
        } else {
            near.push((x, y, colour));
        }
    }

    let mut report = Occlusion::default();
    for (x, y, colour) in &near {
        report.near_samples += 1;
        if drawn_at(rgb, *x, *y, *colour) {
            report.near_drawn += 1;
        }
    }
    for (x, y, colour) in &far {
        // A far-side point can project onto a pixel where the near-side track of the same orbit
        // happens to cross. Those pixels are not evidence either way, so they are skipped.
        if near
            .iter()
            .any(|(other_x, other_y, _)| (other_x - x).hypot(other_y - y) < 8.0)
        {
            continue;
        }
        report.far_samples += 1;
        if drawn_at(rgb, *x, *y, *colour) {
            report.far_violations += 1;
        }
    }
    report
}

/// Whether the pixel at `(x, y)`, or one of its neighbours, has the given colour.
///
/// The neighbourhood is three pixels wide because a projected vertex can land on either side of a
/// pixel boundary, and a one-pixel line is exactly one pixel: an off-by-one sample would otherwise
/// report a visible line as missing.
fn drawn_at(rgb: &[u8], x: f64, y: f64, colour: [u8; 3]) -> bool {
    let cx = (x.round() as isize).clamp(0, isize::try_from(WIDTH - 1).unwrap_or(0));
    let cy = (y.round() as isize).clamp(0, isize::try_from(HEIGHT - 1).unwrap_or(0));
    for dy in -1..=1 {
        for dx in -1..=1 {
            let px = cx + dx;
            let py = cy + dy;
            if px < 0
                || py < 0
                || px >= isize::try_from(WIDTH).unwrap_or(0)
                || py >= isize::try_from(HEIGHT).unwrap_or(0)
            {
                continue;
            }
            let index = (usize::try_from(py).unwrap_or(0) * WIDTH as usize
                + usize::try_from(px).unwrap_or(0))
                * 3;
            let Some(pixel) = rgb.get(index..index + 3) else {
                continue;
            };
            // 24 of 255 is a loose match: a line's colour is written exactly, but a line drawn
            // over the shaded globe may blend at an endpoint.
            if pixel
                .iter()
                .zip(colour.iter())
                .all(|(a, b)| a.abs_diff(*b) <= 24)
            {
                return true;
            }
        }
    }
    false
}

/// Whether the globe hides a point from the camera, by intersecting the line of sight with the
/// sphere.
///
/// `None` means "too close to the limb to call": a sample within 2 % of the radius of the globe's
/// silhouette is neither clearly in front nor clearly behind, and excluding it keeps the check
/// honest rather than lucky.
fn hidden_by_globe(eye: [f64; 3], point: [f64; 3], radius: f64) -> Option<bool> {
    let direction = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]];
    let length_squared = dot(direction, direction);
    if length_squared.is_nan() || length_squared <= 0.0 {
        return None;
    }
    // The parameter of the closest approach of the line of sight to the globe's centre.
    let t = -dot(eye, direction) / length_squared;
    if t <= 0.0 {
        // The closest approach is behind the camera, so nothing is in the way.
        return Some(false);
    }
    if t >= 1.0 {
        // The closest approach is beyond the point itself, so the point is in front.
        return Some(false);
    }
    let closest = [
        eye[0] + t * direction[0],
        eye[1] + t * direction[1],
        eye[2] + t * direction[2],
    ];
    let distance = dot(closest, closest).sqrt();
    if (distance - radius).abs() < 0.02 * radius {
        return None;
    }
    Some(distance < radius)
}

/// The radius of the globe the scene draws, taken from the mesh rather than assumed.
fn globe_radius_km(scene: &SceneData) -> f64 {
    let mut radius: f64 = 0.0;
    for vertex in &scene.globe.vertices {
        radius = radius.max(dot(f64_3(vertex.position), f64_3(vertex.position)).sqrt());
    }
    if radius > 0.0 {
        radius
    } else {
        f64::from(stkcraft_viewport::EARTH_EQUATORIAL_RADIUS_KM)
    }
}

/// A linear colour as the bytes a non-sRGB target holds for it.
fn linear_to_bytes(colour: [f32; 3]) -> [u8; 3] {
    let encode = |linear: f32| -> u8 {
        let value = if linear.is_nan() {
            0.0
        } else {
            linear.clamp(0.0, 1.0)
        };
        (value * 255.0).round() as u8
    };
    [encode(colour[0]), encode(colour[1]), encode(colour[2])]
}

fn f64_3(value: [f32; 3]) -> [f64; 3] {
    [
        f64::from(value[0]),
        f64::from(value[1]),
        f64::from(value[2]),
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Run the occlusion check on a finished frame and report a failure the caller can act on.
///
/// # Errors
///
/// A message naming the numbers when a far-side sample was drawn over the globe (the depth test is
/// not working), or when the near-side control did not draw (the check itself is broken, and its
/// silence about the far side would mean nothing).
fn assert_occlusion(scene: &SceneData, camera: &Camera, rgb: &[u8]) -> Result<(), String> {
    let report = occlusion(scene, camera, rgb);
    eprintln!(
        "depth: {} far-side track sample(s) checked, {} drawn through the globe; control: \
         {}/{} near-side sample(s) drawn",
        report.far_samples, report.far_violations, report.near_drawn, report.near_samples
    );
    if report.far_violations > 0 {
        return Err(format!(
            "{} of {} far-side track samples were drawn over the globe: the depth test is not \
             hiding the far side",
            report.far_violations, report.far_samples
        ));
    }
    if report.far_samples < 20 {
        return Err(format!(
            "only {} far-side track sample(s): the check cannot verify occlusion from this frame",
            report.far_samples
        ));
    }
    // Half of a full-orbit track is in front of the globe; requiring most of those to be drawn is
    // what rules out a sampler that finds nothing and therefore reports no violations.
    let ratio = if report.near_samples == 0 {
        0.0
    } else {
        report.near_drawn as f64 / report.near_samples as f64
    };
    if ratio < 0.9 {
        return Err(format!(
            "{}/{} near-side track samples were drawn, so the occlusion check cannot see the \
             track it is checking",
            report.near_drawn, report.near_samples
        ));
    }
    Ok(())
}

/// Write an 8-bit RGB image as a PNG.
///
/// A PNG is a signature, an IHDR, an IDAT holding zlib-compressed filtered scanlines, and an
/// IEND. The zlib stream uses **stored** (uncompressed) deflate blocks: legal, trivial to emit,
/// and it needs no dependency. The file is larger, which does not matter for a verification image
/// that is never shipped as an asset.
fn write_png<W: std::io::Write>(
    writer: &mut W,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> std::io::Result<()> {
    writer.write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])?;

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // Bit depth 8, colour type 2 (truecolour RGB), deflate, adaptive filtering, no interlace.
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(writer, *b"IHDR", &ihdr)?;

    // Filter type 0 on every scanline, then pack into stored deflate blocks of at most 65 535
    // bytes, wrapped in a zlib header and an Adler-32.
    let stride = (width as usize) * 3;
    let mut raw = Vec::with_capacity((height as usize) * (stride + 1));
    for row in 0..height as usize {
        raw.push(0);
        let start = row * stride;
        let end = start + stride;
        if let Some(line) = rgb.get(start..end) {
            raw.extend_from_slice(line);
        } else {
            raw.extend(std::iter::repeat_n(0, stride));
        }
    }
    write_chunk(writer, *b"IDAT", &zlib_stored(&raw))?;
    write_chunk(writer, *b"IEND", &[])?;
    writer.flush()
}

fn write_chunk<W: std::io::Write>(
    writer: &mut W,
    kind: [u8; 4],
    data: &[u8],
) -> std::io::Result<()> {
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(&kind)?;
    writer.write_all(data)?;
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(&kind);
    crc_input.extend_from_slice(data);
    writer.write_all(&crc32(&crc_input).to_be_bytes())
}

/// A zlib stream made of stored deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut remaining = data;
    loop {
        let take = remaining.len().min(65_535);
        let (block, rest) = remaining.split_at(take);
        let last = rest.is_empty();
        out.push(u8::from(last));
        out.extend_from_slice(&(take as u16).to_le_bytes());
        out.extend_from_slice(&(!(take as u16)).to_le_bytes());
        out.extend_from_slice(block);
        remaining = rest;
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// CRC-32 as IEEE 802.3 defines it, which is what PNG chunks use.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0_u32; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let mut c = index as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        let index = ((crc ^ u32::from(*byte)) & 0xFF) as usize;
        let entry = table.get(index).copied().unwrap_or(0);
        crc = entry ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

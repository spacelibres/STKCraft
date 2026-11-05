//! Rendering one frame offscreen, for verification.
//!
//! The window path cannot be inspected from a build machine with no window system, so this module
//! draws the same [`SceneData`] with the same shader into a texture and writes a PNG. It is how the
//! claim "the viewport works" is checked rather than asserted: the pipeline descriptors, the WGSL
//! compilation, the depth buffer and the geometry all get exercised, and the result is an image
//! that can be looked at.
//!
//! It is switched on by the `STKCRAFT_SCREENSHOT` environment variable and is otherwise never
//! reachable: a normal run opens a window.

use bytemuck::cast_slice;
use stkcraft_viewport::geometry::Vertex;
use stkcraft_viewport::renderer::{self, DEPTH_FORMAT, Globals};
use stkcraft_viewport::scene::SceneData;
use wgpu::util::DeviceExt as _;

use crate::app::App;

/// The texture format used for the offscreen target.
///
/// `Rgba8Unorm` and not `Rgba8UnormSrgb`: the viewport's colours are already linear and the
/// transfer function is applied when the PNG is written, so the format must not apply it too.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The size of the verification image, in pixels.
pub const WIDTH: u32 = 960;
/// See [`WIDTH`].
pub const HEIGHT: u32 = 600;

/// Render the app's current frame to a PNG at `path`.
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

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stkcraft"),
        source: wgpu::ShaderSource::Wgsl(renderer::SHADER.into()),
    });

    let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("globals"),
        contents: bytemuck::bytes_of(&globals),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globals layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals"),
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: globals_buffer.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("stkcraft"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let attributes = vertex_attributes();
    let vertex_buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: Vertex::SIZE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];
    let targets = [Some(wgpu::ColorTargetState {
        format: TARGET_FORMAT,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })];

    // The globe: triangles, depth write on, no blending.
    let globe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("globe"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_globe"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &vertex_buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // The sphere's triangles are counter-clockwise seen from outside; culling the back
            // half is what makes the far side of the Earth hidden rather than smeared over.
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_globe"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    });

    // Lines: depth-tested but not depth-writing, so a track is not z-fought by the surface it
    // sits on. `LineList` with no index buffer, matching how `geometry` produces vertices.
    let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lines"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &vertex_buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    });

    let color_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("colour"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

    eprintln!(
        "camera: {:.0} km from the centre, near {:.0} km, far {:.0} km, {} stars",
        camera.distance_km,
        camera.near_plane_km(),
        camera.far_plane_km(),
        scene.stars.len() / 2
    );
    let globe_buffers = make_mesh_buffers(&device, &scene);

    let star_buffer = make_line_buffer(&device, "stars", &scene.stars);
    let graticule_buffer = make_line_buffer(&device, "graticule", &scene.graticule);
    let track_buffer = make_line_buffer(&device, "tracks", &scene.tracks);
    let marker_buffer = make_line_buffer(&device, "markers", &scene.markers);

    // Reading a texture back needs bytes per row aligned to 256.
    let unpadded_row = WIDTH * 4;
    let padded_row = unpadded_row.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded_row) * u64::from(HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("frame"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Space.
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.016,
                        g: 0.024,
                        b: 0.047,
                        a: 1.0,
                    }),
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
        });

        pass.set_bind_group(0, &bind_group, &[]);

        // The globe first, so that everything else is depth-tested against a real surface.
        pass.set_pipeline(&globe_pipeline);
        if let Some((vertices, indices, index_count)) = &globe_buffers {
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..*index_count, 0, 0..1);
        }

        pass.set_pipeline(&line_pipeline);
        let buffers: [LineBuffer; 4] = [star_buffer, graticule_buffer, track_buffer, marker_buffer];
        for (buffer, count) in buffers.iter().flatten() {
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..*count, 0..1);
        }
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color_texture,
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
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
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

    let mut file = std::fs::File::create(path).map_err(|error| format!("{path}: {error}"))?;
    write_png(&mut file, WIDTH, HEIGHT, &rgb).map_err(|error| format!("{path}: {error}"))?;
    eprintln!(
        "scene at {}: {} track vertices, {} markers",
        epoch_label,
        scene.tracks.len(),
        scene.markers.len()
    );
    Ok(())
}

/// Build the vertex layout the pipelines were created with.
fn vertex_attributes() -> [wgpu::VertexAttribute; 2] {
    use wgpu::VertexFormat;
    let spec = renderer::vertex_attributes();
    [
        wgpu::VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: spec[0].2 as wgpu::BufferAddress,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: spec[1].2 as wgpu::BufferAddress,
            shader_location: 1,
        },
    ]
}

/// The globe's vertex and index buffers, and the index count.
type MeshBuffers = Option<(wgpu::Buffer, wgpu::Buffer, u32)>;

fn make_mesh_buffers(device: &wgpu::Device, scene: &SceneData) -> MeshBuffers {
    let mesh = &scene.globe;
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        return None;
    }
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("globe vertices"),
        contents: cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("globe indices"),
        contents: cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    Some((vertices, indices, mesh.indices.len() as u32))
}

/// One line-list buffer and the number of vertices in it.
type LineBuffer = Option<(wgpu::Buffer, u32)>;

fn make_line_buffer(device: &wgpu::Device, label: &str, vertices: &[Vertex]) -> LineBuffer {
    if vertices.is_empty() {
        return None;
    }
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: cast_slice(vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Some((buffer, vertices.len() as u32))
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

//! Drawing the scene inside `egui`'s own render pass.
//!
//! `egui` is what owns the window's surface, so the viewport cannot open a render pass of its own
//! without a second swapchain and a blit. It does not need to: `egui-wgpu` hands a paint callback
//! the pass it is already recording, with the viewport set to the callback's rectangle and — when
//! the window was created with a depth buffer — a depth attachment cleared for the frame.
//!
//! That depth attachment is the whole reason this module exists. The `egui` painter draws shapes in
//! submission order and has no depth buffer, so a ground track on the far side of the Earth was
//! painted over the disc. Here the globe writes depth and the lines test against it, so the far
//! side of a track is hidden by the Earth the same way the offscreen path hides it.
//!
//! # What is drawn where
//!
//! [`SceneCallback::prepare`] runs before the frame's pass is recorded, which is where the buffers
//! are filled. [`SceneCallback::paint`] runs inside the pass, which is where the draws are
//! recorded. Both go through [`SceneRenderer`], the same type the offscreen verification path
//! uses, so the window and the screenshot cannot drift apart.

use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use wgpu::{CommandBuffer, Device, Queue, RenderPass};

use crate::camera::Camera;
use crate::renderer::{DEPTH_FORMAT, Globals, SceneRenderer};
use crate::scene::SceneData;

/// One frame of scene, registered with `egui` as a paint callback.
pub struct SceneCallback {
    scene: SceneData,
    view: crate::camera::Mat4,
    projection: crate::camera::Mat4,
    target_format: wgpu::TextureFormat,
}

impl SceneCallback {
    /// Build the callback for a frame seen from a camera in a viewport of a given aspect ratio.
    ///
    /// `aspect` is width over height of the rectangle the callback is registered for, in points or
    /// in pixels — the ratio is the same either way, which is why the caller can pass the `egui`
    /// response's rectangle before the pixels-per-point scale is known. A degenerate or
    /// non-finite aspect is replaced with `1.0` rather than propagated into the projection matrix,
    /// where it would produce a singular matrix and an empty view instead of a square one.
    ///
    /// `target_format` is the surface format the pipelines must be created for, from
    /// [`eframe::Frame::wgpu_render_state`]`().target_format`.
    #[must_use]
    pub fn new(
        scene: SceneData,
        camera: &Camera,
        aspect: f32,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            scene,
            view: camera.view(),
            projection: camera.projection(sanitized_aspect(aspect)),
            target_format,
        }
    }
}

/// An aspect ratio that a projection matrix can use: positive and finite, or `1.0`.
#[must_use]
pub fn sanitized_aspect(aspect: f32) -> f32 {
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    }
}

impl CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        let renderer = callback_resources
            .entry::<SceneRenderer>()
            .or_insert_with(|| SceneRenderer::new(device, self.target_format, DEPTH_FORMAT));

        // The surface format can change underneath a running window — dragging it to a monitor
        // driven by another GPU is the usual way — and a pipeline built for the old format makes
        // every frame a validation error. Rebuilding is a handful of microseconds and happens
        // once per change.
        if renderer.target_format() != self.target_format {
            *renderer = SceneRenderer::new(device, self.target_format, DEPTH_FORMAT);
        }

        renderer.upload(
            device,
            queue,
            &Globals::new(self.view, self.projection),
            &self.scene,
        );

        // Nothing was recorded into the shared encoder: the draws happen in `paint`, inside the
        // pass that already carries the colour and depth attachments.
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        // `prepare` always inserts the renderer for this frame, so a miss would mean `egui` called
        // `paint` without `prepare`; nothing to draw is the safe answer to that.
        if let Some(renderer) = callback_resources.get::<SceneRenderer>() {
            // The pass's viewport already covers the callback's rectangle, which is all the
            // shader's clip-space output needs; the scissor is `egui`'s clip rectangle.
            renderer.draw(render_pass);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_aspect_becomes_a_square_viewport() {
        // A zero-height rectangle arrives during window resizing, and a NaN one from a bad
        // `Response`; both would be a singular projection instead of a usable frame.
        assert_eq!(sanitized_aspect(1.6), 1.6);
        assert_eq!(sanitized_aspect(0.0), 1.0);
        assert_eq!(sanitized_aspect(-2.0), 1.0);
        assert_eq!(sanitized_aspect(f32::NAN), 1.0);
        assert_eq!(sanitized_aspect(f32::INFINITY), 1.0);
    }

    #[test]
    fn the_callback_carries_the_cameras_matrices_and_the_frames_geometry() {
        let frame = crate::frame::Frame::empty("test");
        let camera = Camera::default_view();
        let scene = SceneData::build(&frame, &camera);
        let callback =
            SceneCallback::new(scene.clone(), &camera, 2.0, wgpu::TextureFormat::Bgra8Unorm);
        // The projection must be the one for the aspect it was given, not the default.
        assert_eq!(
            callback.projection,
            camera.projection(2.0),
            "the callback used a different projection than the camera"
        );
        assert_eq!(callback.view, camera.view());
        assert_eq!(callback.scene, scene);
    }
}

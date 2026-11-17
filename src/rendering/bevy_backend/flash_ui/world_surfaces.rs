//! 4b0bc0 -> 02bfd0 -> 2369c0: a movie's render target is sampled
//! by its model. Host: retain Bevy Image handles across extraction and encode
//! the same GDraw commands before scene passes, rather than drawing an overlay.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlashWorldKey {
    pub entity: crate::gameplay::entity_lifetime::EntityId,
    pub handle: u64,
    pub generation: u32,
}

pub struct FlashWorldAcknowledgement {
    admitted: AtomicBool,
    rendered: AtomicU64,
    rendered_valid: AtomicBool,
    retired: AtomicBool,
}
impl Default for FlashWorldAcknowledgement {
    fn default() -> Self {
        Self { admitted: AtomicBool::new(false), rendered: AtomicU64::new(0), rendered_valid: AtomicBool::new(false), retired: AtomicBool::new(false) }
    }
}
impl FlashWorldAcknowledgement {
    /// A render owner exists even if the image has not finished GPU preparation.
    pub fn admitted(&self) -> bool { self.admitted.load(Ordering::Acquire) }
    /// The actual list has been encoded into its prepared target.
    pub fn rendered(&self, serial: u64) -> bool {
        self.rendered_valid.load(Ordering::Acquire)
            && self.rendered.load(Ordering::Acquire) == serial
    }
    /// The render owner has withdrawn this key and dropped its mutable state.
    /// Submitted commands retain the GPU leases they use until completion.
    pub fn retired(&self) -> bool { self.retired.load(Ordering::Acquire) }
}

#[derive(Clone)]
pub struct FlashWorldFrame {
    pub frame: FlashFrame,
    /// Rgba8UnormSrgb with Rgba8Unorm in view_formats, TEXTURE_BINDING |
    /// RENDER_ATTACHMENT, real movie dimensions. The material samples SRGB;
    /// the renderer writes the raw UNORM view of those same bytes.
    pub target: Handle<bevy::prelude::Image>,
    pub ack: Arc<FlashWorldAcknowledgement>,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct FlashWorldFrames(pub HashMap<FlashWorldKey, FlashWorldFrame>);

pub(super) struct WorldGpu {
    movie: MovieGpu,
    target: bevy::asset::AssetId<bevy::prelude::Image>,
    ack: Arc<FlashWorldAcknowledgement>,
    rendered: Option<(u64, u64, TextureId)>,
    draw_view: Option<(TextureId, TextureView)>,
}
impl Drop for WorldGpu {
    fn drop(&mut self) {
        self.movie = MovieGpu::default();
        self.draw_view = None;
        self.ack.retired.store(true, Ordering::Release);
    }
}

/// Runs even when no camera renders: a closed component must not wait for a
/// view to release its Flash buffers and acknowledge image-handle retirement.
pub(super) fn synchronize(frames: Res<FlashWorldFrames>, mut gpu: ResMut<FlashUiGpu>) {
    gpu.worlds.retain(|key, state| frames.0.get(key).is_some_and(|frame|
        state.target == frame.target.id() && Arc::ptr_eq(&state.ack, &frame.ack)));
    for (key, frame) in &frames.0 {
        gpu.worlds.entry(*key).or_insert_with(|| WorldGpu {
            movie: MovieGpu::default(), target: frame.target.id(),
            ack: frame.ack.clone(), rendered: None, draw_view: None,
        });
        frame.ack.admitted.store(true, Ordering::Release);
    }
    gpu.common.textures.retire_unowned(&[]);
}

pub(super) fn render(
    frames: Res<FlashWorldFrames>, device: Res<RenderDevice>, queue: Res<RenderQueue>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    mut gpu: ResMut<FlashUiGpu>, mut context: RenderContext,
) {
    let FlashUiGpu { common, worlds, .. } = &mut *gpu;
    for (key, frame) in &frames.0 {
        let Some(state) = worlds.get_mut(key) else { continue };
        let Some(target) = images.get(frame.target.id()) else { continue };
        // A format mismatch is not a successful render or readiness ACK.
        if target.texture_descriptor.format != TextureFormat::Rgba8UnormSrgb
            || !target.texture_descriptor.usage.contains(TextureUsages::RENDER_ATTACHMENT)
            || !target.texture_descriptor.view_formats.contains(&UI_FORMAT) {
            continue;
        }
        let identity = (frame.frame.generation, frame.frame.serial, target.texture.id());
        // Registered game images may change without rebuilding the Flash list.
        let dynamic_fill=frame.frame.list.batches.iter().any(|batch|matches!(&batch.texture,FillTexture::Game(_)));
        if state.rendered == Some(identity) && !dynamic_fill { continue; }
        // 02c010's packed0x58a -> 2c2f50 is default SRGB(DXGI29).
        // Its descriptor+40 flag sets image+74; 2f4060 then creates TYPELESS
        // storage. 31a600 forces interpretation5 -> UNORM(DXGI28) for GDraw
        // through 2e9d10 -> 2dd580 -> 2bd230. The model's default+98 sample
        // view retains SRGB. Host: Bevy authorizes the equivalent raw view.
        if state.draw_view.as_ref().is_none_or(|(id,_)|*id!=target.texture.id()) {
            let view=target.texture.create_view(&TextureViewDescriptor {
                label:Some("flash world raw write"),format:Some(UI_FORMAT),..Default::default()
            });
            state.draw_view=Some((target.texture.id(),view));
        }
        let draw_view=&state.draw_view.as_ref().unwrap().1;
        if render_frame(&frame.frame, &device, &queue, common, &mut state.movie,
            &images, context.command_encoder(), target.texture_descriptor.size,
            draw_view, None) {
            state.rendered = Some(identity);
            state.ack.rendered.store(frame.frame.serial, Ordering::Release);
            state.ack.rendered_valid.store(true, Ordering::Release);
        }
    }
}

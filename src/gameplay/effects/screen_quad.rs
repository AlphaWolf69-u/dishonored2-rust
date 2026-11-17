//! Fullscreen quad21d5f0/21d4a0, using the canonical standard scalar envelope.
use super::environment::EnvironmentBlendEffect;
use crate::assets::fx::screen_quad::ScreenQuadAsset;
use std::sync::Arc;

pub(crate) struct Runtime {
    pub asset: Arc<ScreenQuadAsset>,
    pub blend: EnvironmentBlendEffect,
}
impl Runtime {
    pub fn new(
        asset: Arc<ScreenQuadAsset>,
        offsets: [f32; 4],
        view_mask: u8,
        seed: &mut u32,
    ) -> Result<Self, String> {
        let mut blend = EnvironmentBlendEffect::new(asset.blend.clone(), offsets, view_mask, seed)?;
        //21d5f0 copies the initialized fade-in value into the registered
        //quad's current blend before its first21d6e0 update.
        blend.blend = blend.last_main;
        Ok(Self { asset, blend })
    }
}
#[derive(Clone)]
pub struct Draw {
    ///The same immutable declaration generation, including material and stage.
    ///The renderer resolves its already prepared material by this identity.
    pub asset: Arc<ScreenQuadAsset>,
    pub blend: f32,
}
///Family19a3f0 has32 slots. Stage publication preserves registration order;
///the view's stage-list owner supplies its independently sourced admission cap.
pub struct Frame {
    pub draws: [Option<Draw>; 32],
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            draws: std::array::from_fn(|_| None),
        }
    }
}
impl Frame {
    pub fn iter(&self) -> impl Iterator<Item = &Draw> {
        self.draws.iter().filter_map(Option::as_ref)
    }
}

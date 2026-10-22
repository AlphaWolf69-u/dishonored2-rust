//! Persistent targets for the source bloom schedule. View-coordinate constants
//! remain explicit inputs from the frame owner, as in the other raster passes.
use super::*;
use crate::rendering::bloom::BloomSchedule;

#[derive(Clone, Copy)]
pub struct BloomViewParameters {
    pub pixel_scale: Vec4,
    pub viewport: Vec4,
}
impl BloomViewParameters {
    /// 325d80 sets a zero-origin viewport covering each complete target.
    pub fn full_target(extent: [u32; 2], resolution_scale: [f32; 2], subpixel: [f32; 2]) -> Self {
        let dimensions = extent.map(|v| i32::try_from(v).expect("bloom target dimension"));
        let view = crate::rendering::viewport::parameters([0, 0], dimensions, dimensions);
        Self {
            pixel_scale: Vec4::from_array(view.position_to_view_texture),
            viewport: Vec4::new(
                resolution_scale[0],
                resolution_scale[1],
                subpixel[0],
                subpixel[1],
            ),
        }
    }
}

pub struct BloomChain {
    schedule: BloomSchedule,
    steps: Vec<Step>,
}
struct Step {
    texture: Texture,
    view: TextureView,
    pass: BloomUpsamplePass,
}

impl BloomChain {
    pub fn new(device: &RenderDevice, schedule: BloomSchedule) -> Result<Self, String> {
        if schedule.draws.is_empty() {
            return Err("enabled bloom needs at least one draw".into());
        }
        for (index, draw) in schedule.draws.iter().enumerate() {
            if draw
                .source_extent
                .into_iter()
                .chain(draw.target_extent)
                .any(|v| v == 0)
                || (index > 0 && draw.source_extent != schedule.draws[index - 1].target_extent)
            {
                return Err("bloom schedule has inconsistent extents".into());
            }
        }
        let pipeline = BloomUpsamplePass::new(device, TextureFormat::Rgba16Float);
        let steps = schedule
            .draws
            .iter()
            .map(|draw| {
                let texture = device.create_texture(&TextureDescriptor {
                    label: Some("bloom intermediate"),
                    size: Extent3d {
                        width: draw.target_extent[0],
                        height: draw.target_extent[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba16Float,
                    usage: TextureUsages::TEXTURE_BINDING
                        | TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&TextureViewDescriptor::default());
                Step {
                    texture,
                    view,
                    pass: pipeline.sibling(),
                }
            })
            .collect();
        Ok(Self { schedule, steps })
    }

    pub fn schedule(&self) -> &BloomSchedule {
        &self.schedule
    }
    pub fn output(&self) -> &TextureView {
        &self.steps.last().unwrap().view
    }
    pub fn output_texture(&self) -> &Texture {
        &self.steps.last().unwrap().texture
    }
    #[cfg(test)]
    pub(super) fn test_textures(&self) -> impl Iterator<Item = &Texture> {
        self.steps.iter().map(|step| &step.texture)
    }

    /// Record all H/V draws into the caller's encoder. Every draw owns its
    /// uniform buffer, so queued parameter writes cannot replace prior draws.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        views: &[BloomViewParameters],
        veil_size: f32,
    ) -> Result<(), String> {
        if views.len() != self.steps.len() {
            return Err("bloom needs view constants for every scheduled draw".into());
        }
        let mut input = source.clone();
        for ((step, draw), view) in self.steps.iter_mut().zip(&self.schedule.draws).zip(views) {
            step.pass.record(
                device,
                queue,
                encoder,
                BloomParameters {
                    pixel_scale: view.pixel_scale,
                    viewport: view.viewport,
                    blur: Vec4::new(draw.delta[0], draw.delta[1], veil_size, 0.0),
                },
                &input,
                &step.view,
            );
            input = step.view.clone();
        }
        Ok(())
    }
}

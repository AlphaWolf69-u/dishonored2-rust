//! Per-draw alpha-fog constants selected by 324be0, indexed by mesh tags.
use super::fog::AreaFogFrame;
use bevy::{
    prelude::*,
    render::{render_resource::*, renderer::RenderQueue, storage::ShaderBuffer},
};

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct FogObjectParameters {
    pub distance: Vec4,
    pub color_start: Vec4,
    pub color_end: Vec4,
    pub density: Vec4,
    pub scattering_direction_phase: Vec4,
    pub scattering_color_intensity: Vec4,
    pub volume_type: Vec4,
    pub interval: Vec4,
}
impl FogObjectParameters {
    pub const SIZE: usize = 128;
    pub fn selected(area: &AreaFogFrame, depths: [f32; 3]) -> Self {
        let p = &area.parameters;
        Self {
            distance: p.distance,
            color_start: p.color_start.extend(0.),
            color_end: p.color_end.extend(0.),
            density: Vec4::new(p.density_factor, p.falloff, 0., 0.),
            scattering_direction_phase: p.scattering_direction.extend(p.scattering_phase),
            scattering_color_intensity: p.scattering_color.extend(p.scattering_intensity),
            volume_type: p.volume_type.extend(0.),
            interval: Vec4::new(depths[0], depths[1], depths[2], 0.),
        }
    }
}

#[derive(Clone)]
pub struct FogObjectFrame {
    pub camera: Entity,
    pub revision: u64,
    pub world_offset: u32,
    pub capacity: usize,
    /// Sorted, unique updates. Missing selection is an explicit zero record;
    /// invisible objects are never read and need no upload.
    pub updates: Vec<(u32, FogObjectParameters)>,
}
impl FogObjectFrame {
    pub fn write(&self, queue: &RenderQueue, buffer: &Buffer) -> Result<(), String> {
        if buffer.size() < (self.capacity * FogObjectParameters::SIZE) as u64 {
            return Err("alpha fog object buffer awaits current tag domain".into());
        }
        if self.updates.is_empty() {
            return Ok(());
        }
        let data = ShaderBuffer::from(self.updates.iter().map(|(_, p)| *p).collect::<Vec<_>>());
        let data = data.data.as_ref().unwrap();
        let mut first = 0;
        while first < self.updates.len() {
            let mut last = first + 1;
            while last < self.updates.len() && self.updates[last].0 == self.updates[last - 1].0 + 1
            {
                last += 1;
            }
            let index = self.updates[first].0 as usize;
            if index + (last - first) > self.capacity {
                return Err("alpha fog receiver exceeds tag domain".into());
            }
            queue.write_buffer(
                buffer,
                (index * FogObjectParameters::SIZE) as u64,
                &data[first * FogObjectParameters::SIZE..last * FogObjectParameters::SIZE],
            );
            first = last;
        }
        Ok(())
    }
}

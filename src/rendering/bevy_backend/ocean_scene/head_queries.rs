//! Swimming's five samples share the actual gameplay ocean simulation.
use super::*;
use crate::rendering::bevy_backend::ocean_height_queries::{
    OceanHeightQueries, OceanHeightTextures, OceanQueryReadbacks,
};

#[derive(Debug)]
pub(crate) struct HeadWaterSnapshot {
    pub frame: u32,
    pub plane: Option<f32>,
    pub surface: Option<f32>,
    pub body_surface: Option<f32>,
    pub points: Option<[[f32; 2]; 5]>,
    pub samples: Option<[[f32; 4]; 5]>,
}
pub(super) struct HeadQueries {
    queries: OceanHeightQueries,
    requests: OceanQueryQueue,
    readbacks: OceanQueryReadbacks,
    handles: [OceanQueryHandle; 4],
    points: [Option<[[f32; 2]; 5]>; 4],
    cursor: usize,
    ring: usize,
    texture_ids: [TextureViewId; 2],
    pub snapshot: Option<HeadWaterSnapshot>,
}
impl HeadQueries {
    fn new(device: &RenderDevice, textures: OceanHeightTextures<'_>) -> Self {
        let texture_ids = [textures.shore_distance.id(), textures.amplitude_mask.id()];
        Self {
            texture_ids,
            queries: OceanHeightQueries::new(device, textures),
            requests: OceanQueryQueue::new(),
            readbacks: OceanQueryReadbacks::new(device),
            handles: [OceanQueryHandle::INVALID; 4],
            points: [None; 4],
            cursor: 0,
            ring: 0,
            snapshot: None,
        }
    }
    fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &WaterFrame,
        surface: &OceanSurfaceParameters,
        water_level: f32,
        wave_slot: usize,
    ) -> Result<(), String> {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.frame == frame.matrices.frame)
        {
            return Ok(());
        }
        self.readbacks
            .read_current_mapped(device, self.ring, &self.requests)?;
        let mut snapshot = HeadWaterSnapshot {
            frame: frame.matrices.frame,
            plane: frame.head_water_surface,
            surface: frame.head_water_surface,
            body_surface: None,
            points: None,
            samples: None,
        };
        if frame.head_water_surface.is_some() {
            let mut samples = [[0.; 4]; 5];
            if let Some(count) = self.requests.take(
                &mut self.handles[self.cursor],
                self.readbacks.completed_rings(),
                &mut samples,
            )? {
                if count != 5 {
                    return Err("swimming wave query did not return five points".into());
                }
                // 88b451 stores the HEAD sample before 88b4a0's MAXSS loop.
                // The maximum drives body buoyancy, not the head-underwater flag.
                snapshot.surface = Some(samples[0][0]);
                snapshot.body_surface =
                    Some(crate::gameplay::player::swimming::highest_wave(&samples));
                snapshot.samples = Some(samples);
                snapshot.points = self.points[self.cursor].take();
            }
            if let Some(points) = frame.head_wave_points {
                // Gameplay registration sees the preceding renderer ring.
                // 3032b0 assigns that ring+1, the ring submitted below.
                let handle = self.requests.register((self.ring + 3) & 3, &points)?;
                self.handles[self.cursor] = handle;
                self.points[self.cursor] = Some(points);
                if !handle.is_invalid() {
                    self.cursor = (self.cursor + 1) & 3;
                }
            }
        }
        self.queries.record_pending(
            queue,
            encoder,
            &mut self.requests,
            &mut self.readbacks,
            surface,
            water_level,
            wave_slot,
            self.ring,
        )?;
        self.ring = (self.ring + 1) & 3;
        self.snapshot = Some(snapshot);
        Ok(())
    }
}
impl OceanScene {
    pub(super) fn record_head_queries(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &WaterFrame,
    ) -> Result<(), String> {
        if self.head_queries.is_some() || frame.head_wave_points.is_some() {
            let settings = &frame.ocean_environment.surface;
            let textures = OceanHeightTextures {
                noise: &self.noise,
                displacement: self.waves.query_displacement_views(),
                collision_height: self.waves.collision_height_view(),
                foam: self.waves.foam_view(),
                shore_distance: &self
                    .images
                    .get(&settings.shore_distance_image)
                    .ok_or("head wave shore mask absent")?
                    .view,
                amplitude_mask: &self
                    .images
                    .get(&settings.wave_amplitude_image)
                    .ok_or("head wave amplitude mask absent")?
                    .view,
            };
            let texture_ids = [textures.shore_distance.id(), textures.amplitude_mask.id()];
            if let Some(queries) = self.head_queries.as_mut() {
                if queries.texture_ids != texture_ids {
                    queries.queries.rebind_textures(device, textures);
                    queries.texture_ids = texture_ids;
                }
            } else {
                self.head_queries = Some(HeadQueries::new(device, textures));
            }
        }
        if let Some(queries) = self.head_queries.as_mut() {
            queries.record(
                device,
                queue,
                encoder,
                frame,
                self.surface
                    .as_ref()
                    .ok_or("head wave surface parameters absent")?,
                self.state.view.water_level,
                self.waves.current_slot(),
            )?;
        }
        Ok(())
    }
    pub(crate) fn map_head_queries(&mut self) -> Result<(), String> {
        if let Some(queries) = self.head_queries.as_mut() {
            queries.readbacks.map_submitted()?;
        }
        Ok(())
    }
    pub(crate) fn head_water_snapshot(&self, frame: u32) -> Option<&HeadWaterSnapshot> {
        self.head_queries
            .as_ref()?
            .snapshot
            .as_ref()
            .filter(|s| s.frame == frame)
    }
}

//! Projection geometry and WORLD target selection used by 298ee0.
//! Material variants, runtime decal ownership and draw submission are separate.
use super::{bounds::OrientedBox, shadows::local_caster_query::LocalWorldQuery};

mod pool;
pub use pool::{DecalPool, DecalSlot};

pub struct WorldProjection {
    pub bounds: [[f32; 3]; 2],
    /// 10a920 preserves the previous matrix if inversion fails.
    pub inverse_updated: bool,
    /// Sorted WORLD instance indices, independent of this frame's visibility.
    pub targets: Vec<u32>,
}

/// Refresh the dirty projection's matrix, bounds and affected WORLD records.
/// `world_to_decal` is retained owner state: 10a920 leaves it unchanged for a
/// singular transform, rather than substituting an identity or zero matrix.
pub fn refresh_world_projection(
    tree: &LocalWorldQuery,
    position: [f32; 3],
    axes: [[f32; 3]; 3],
    scale: [f32; 3],
    world_to_decal: &mut [[f32; 4]; 4],
) -> WorldProjection {
    let local = [[-1., -1., -1.], [1., 1., 0.]];
    let transform = super::render_model::affine_rows(position, axes, scale);
    let inverse_updated = super::render_shadow::invert_rows(&transform, world_to_decal);
    let bounds = super::render_model::transformed_bounds(local, position, axes, scale);
    // 298ee0 computes a scaled box centre before rotating it. In particular,
    // projection depth runs from -scale.z to zero, rather than +/-scale.z.
    let centre =
        std::array::from_fn::<_, 3, _>(|i| (local[1][i] * scale[i] + local[0][i] * scale[i]) * 0.5);
    let volume = OrientedBox {
        centre: std::array::from_fn(|i| {
            ((centre[1] * axes[1][i] + centre[0] * axes[0][i]) + centre[2] * axes[2][i])
                + position[i]
        }),
        half_extent: std::array::from_fn(|i| local[1][i] * scale[i] - centre[i]),
        axes,
    };
    let targets = tree
        .query_box(bounds, &volume)
        .into_iter()
        .enumerate()
        .filter_map(|(i, hit)| (hit != 0).then_some(i as u32))
        .collect();
    WorldProjection {
        bounds,
        targets,
        inverse_updated,
    }
}

/// The fields copied from a dynamic record by 1c00d0. Material resources remain
/// owned by their installed-declaration loader, rather than a pointer ABI.
#[derive(Clone)]
pub struct DecalDescription<M> {
    pub position: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub scale: [f32; 3],
    pub colour: [f32; 4],
    pub material: M,
}

pub struct CommittedDecal<M> {
    pub description: DecalDescription<M>,
    pub world_to_decal: [[f32; 4]; 4],
    pub projection: WorldProjection,
}
impl<M: Clone> CommittedDecal<M> {
    fn new(description: &DecalDescription<M>, tree: &LocalWorldQuery) -> Result<Self, String> {
        let mut world_to_decal = [[0.; 4]; 4];
        let projection = refresh_world_projection(
            tree,
            description.position,
            description.axes,
            description.scale,
            &mut world_to_decal,
        );
        // The executable leaves constructor matrix storage unwritten. Never
        // publish a fabricated transform if the first inversion is singular.
        if !projection.inverse_updated {
            return Err("initial decal projection transform is singular".into());
        }
        Ok(Self {
            description: description.clone(),
            world_to_decal,
            projection,
        })
    }

    fn update(&mut self, description: &DecalDescription<M>, tree: &LocalWorldQuery) {
        //09b460 compares all nine basis floats exactly; colour/material changes
        //do not invalidate WORLD intersections. NaNs remain unequal.
        if self.description.position != description.position
            || self.description.axes != description.axes
            || self.description.scale != description.scale
        {
            self.projection = refresh_world_projection(
                tree,
                description.position,
                description.axes,
                description.scale,
                &mut self.world_to_decal,
            );
        }
        self.description = description.clone();
    }
}

impl<M: Clone> DecalPool<DecalDescription<M>, CommittedDecal<M>> {
    ///1c00d0 publishes in physical slot order, not allocation-age order. Reuse
    ///committed objects when a full pool replaces the oldest FX owner.
    pub fn prepare_world(&mut self, tree: &LocalWorldQuery) -> Result<Vec<u32>, String> {
        let mut indices = Vec::with_capacity(self.alive().len());
        for index in 0..self.limit() as u32 {
            let slot = self.slot_mut(index).unwrap();
            if slot.owner().is_none() {
                continue;
            }
            let description = slot
                .description
                .as_ref()
                .ok_or("live decal has no authored description")?;
            if let Some(renderer) = slot.renderer.as_mut() {
                renderer.update(description, tree);
            } else {
                slot.renderer = Some(CommittedDecal::new(description, tree)?);
            }
            indices.push(index);
        }
        Ok(indices)
    }
}

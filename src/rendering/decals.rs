//! Projection geometry and WORLD target selection used by 298ee0.
//! Material variants, runtime decal ownership and draw submission are separate.
use super::{bounds::OrientedBox, shadows::local_caster_query::LocalWorldQuery};

pub struct WorldProjection {
    pub bounds: [[f32; 3]; 2],
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
    super::render_shadow::invert_rows(&transform, world_to_decal);
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
    WorldProjection { bounds, targets }
}

//! 31ad30's type-2 list acceptance, with ordinary material categories disabled.
//! The caller owns the prepared model flags and optional Tome query result.
use super::{
    distance_fade, in_model_region, model_eligible, transformed_aabb_clip_cull, ModelQueryInput,
};

#[derive(Clone, Copy)]
pub struct ModelFadeInput {
    pub camera: [f32; 3],
    pub fov_radians: f32,
    pub visible_range: f32,
    pub lod_scale: f32,
}

/// The ordinary gather's screen-size fade. This is independent of its hard
/// model region and projected-area rejection. A camera inside the model
/// bounds bypasses the fade before this branch is evaluated.
fn fade_visible(bounds: [[f32; 3]; 2], input: ModelFadeInput) -> bool {
    if (0..3).all(|i| bounds[0][i] <= input.camera[i] && input.camera[i] <= bounds[1][i]) {
        return true;
    }
    let center: [f32; 3] = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
    let half: [f32; 3] = std::array::from_fn(|i| (bounds[1][i] - bounds[0][i]) * 0.5);
    let mut extent = half[1];
    if extent <= half[2] {
        extent = half[2];
    }
    if half[0] > extent {
        extent = half[0];
    }
    let range = if input.visible_range <= 0. {
        extent * input.lod_scale * 6.
    } else {
        input.visible_range
    };
    let delta: [f32; 3] = std::array::from_fn(|i| input.camera[i] - center[i]);
    let mut square = delta[1] * delta[1] + delta[0] * delta[0] + delta[2] * delta[2];
    if square <= f32::MIN_POSITIVE {
        square = f32::MIN_POSITIVE;
    }
    let inverse = 1. / square.sqrt();
    let direction = delta.map(|v| v * inverse);
    let step: [f32; 3] = std::array::from_fn(|i| half[i] / (0.000001 + direction[i].abs()));
    let step = step[2].min(step[1]).min(step[0]);
    let gap: [f32; 3] =
        std::array::from_fn(|i| (direction[i] * step + center[i]) - input.camera[i]);
    let tangent = (input.fov_radians * 0.5).tan();
    let square = (gap[1] * gap[1] + gap[0] * gap[0] + gap[2] * gap[2]) * (tangent * tangent);
    square < 0. || distance_fade(range, range * 0.25, square.sqrt()) > 0.
}

/// Determine area-model membership before 307530 sorts the local fog list.
/// `tome` is the actual A764c0 result and its projected area, not colour-mesh
/// visibility. The no-Tome path uses the original projected-box helpers.
pub fn area_model_visible(
    model: ModelQueryInput,
    camera: [f32; 3],
    viewport: [u32; 2],
    clip_rows: [[f32; 4]; 4],
    tome: Option<(bool, f32)>,
    // None represents the model's explicit +1c1c bit5 fade bypass.
    fade: Option<ModelFadeInput>,
) -> bool {
    if !model_eligible(model.frame_flags, model.shape_flags)
        || !in_model_region(camera, model.region)
    {
        return false;
    }
    let minimum_area = 25. / (viewport[0] as f32 * viewport[1] as f32);
    let bypass = model.shape_flags & 0x10 != 0 && model.frame_flags & 1 == 0;
    let area_pass = if let Some((visible, area)) = tome {
        // COMISS/JBE accepts unordered areas too; the decompiler's <= loses
        // that branch. Keep the rejection as an ordered greater-than test.
        if visible
            && (model.frame_flags & 0x40 != 0 || !(minimum_area > area))
            && fade.is_none_or(|input| fade_visible(model.bounds, input))
        {
            return true;
        }
        false
    } else {
        let projected = crate::render_shadow::projected_box_bounds(&clip_rows, model.bounds, false);
        let area = (projected[1][0] - projected[0][0]) * (projected[1][1] - projected[0][1]);
        model.frame_flags & 0x40 != 0 || !(area * 0.5 < minimum_area)
    };
    if !area_pass && !bypass {
        return false;
    }
    let flat = std::array::from_fn(|i| clip_rows[i / 4][i % 4]);
    !transformed_aabb_clip_cull(&flat, model.bounds[0], model.bounds[1]).culled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_list_membership_matches_installed_gather() {
        let identity = std::array::from_fn(|r| std::array::from_fn(|c| u8::from(r == c) as f32));
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/fog_volume_gather.txt").lines() {
            let v: Vec<f32> = line
                .split_whitespace()
                .map(|s| s.parse().unwrap())
                .collect();
            assert_eq!(v.len(), 11);
            let model = ModelQueryInput {
                frame_flags: v[1] as u8,
                shape_flags: v[2] as u8,
                region: [0., 0., 0., 1000.],
                bounds: [[-0.25, -0.25, v[5]], [0.25, 0.25, v[5] + 0.25]],
            };
            assert_eq!(
                area_model_visible(
                    model,
                    [0.; 3],
                    [100; 2],
                    identity,
                    (v[0] != 0.).then_some((v[3] != 0., v[4])),
                    (v[7] != 0.).then_some(ModelFadeInput {
                        camera: [0.; 3],
                        fov_radians: v[8],
                        visible_range: v[9],
                        lod_scale: v[10],
                    })
                ),
                v[6] != 0.,
                "gather case{count}: {line}",
            );
            count += 1;
        }
        assert_eq!(count, 1152);
    }
}

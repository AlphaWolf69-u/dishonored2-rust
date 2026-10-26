//! Water/alpha depth dispatch from 2fe850 and its 2fd8f0 water draw wrapper.
//! Resource readiness, shader ABI, rectangles and actual draws remain with the
//! GPU owner. This iterator does not manufacture completed depth or ocean data.

pub mod parameters;
pub mod program;
pub mod rays;
mod settings;
pub use settings::{FLOAT_PARAMETER_NAMES, IMAGE_PARAMETER_NAMES, CausticSettings, WaterReflectionSettings, WaterSettings, WaterWaveSettings};

/// 3045a0's per-view water/wsRefractedLightDir update. Inputs are the actual
/// parallel-light direction. Preserve the source's separate rounded constants
/// and its zero products, including propagation from nonfinite input lanes.
pub fn refracted_light_direction(light: [f32; 3]) -> [f32; 3] {
    let incident = light.map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000));
    let dot = (incident[0] * 0.0 + incident[1] * 0.0) + incident[2] * 1.0;
    let cosine = f32::from_bits(dot.to_bits() ^ 0x8000_0000);
    let eta = f32::from_bits(0x3f40_0c4b);
    let squared_eta = f32::from_bits(0x3f10_1271);
    let k = 1.0 - (1.0 - cosine * cosine) * squared_eta;
    // With the source's fixed up normal and eta<1, k is nonnegative or NaN.
    // The native COMISS/JA also takes the square-root path for unordered k.
    let t = cosine * eta - k.sqrt();
    [
        incident[0] * eta + t * 0.0,
        incident[1] * eta + t * 0.0,
        incident[2] * eta + t * 1.0,
    ]
}

#[derive(Clone, Copy, Debug)]
pub struct WaterDepthInputs {
    /// View+3a36 bit4 selects the alternate copy/alpha/coarse-only branch.
    pub alternate_path: bool,
    /// Result of 302d20, supplied by the actual ocean owner.
    pub ocean_active: bool,
    /// The current view environment's water/IsWaterVolume value.
    pub water_volume: bool,
    pub world_water: bool,
    pub model_water: bool,
    pub world_boats: bool,
    pub model_boats: bool,
    /// Native task+68. A nonempty world list alone does not imply its owner.
    pub world_renderer_present: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WaterDepthStep {
    ClearWaterStencil,
    CopyOpaqueDepth,
    BoatDepth,
    BoatStencilWater,
    OceanDepth,
    PrepareWaterParameters,
    WorldWaterDepth,
    ModelWaterDepth,
    RestoreBoatDepth,
    PrepareAlphaDepth,
    BoatStencilReduced,
    BoatStencilMain,
    UnderwaterMask,
    PrepareCoarseDepth,
}

/// After any required source-depth decompression, preserve native dispatch
/// order without allocating a per-frame command vector. A step denotes a pass
/// invocation; it does not imply that a pass with empty geometry changes state.
pub fn water_depth_steps(input: WaterDepthInputs) -> impl Iterator<Item = WaterDepthStep> {
    use WaterDepthStep::*;
    let normal = !input.alternate_path;
    let water = input.world_water || input.model_water;
    let draw_water = normal && (input.ocean_active || water);
    let boats = input.world_boats || input.model_boats;
    [
        normal.then_some(ClearWaterStencil),
        Some(CopyOpaqueDepth),
        (draw_water && boats).then_some(BoatDepth),
        (draw_water && boats).then_some(BoatStencilWater),
        (draw_water && input.ocean_active).then_some(OceanDepth),
        (draw_water && water).then_some(PrepareWaterParameters),
        (draw_water && input.world_renderer_present && input.world_water)
            .then_some(WorldWaterDepth),
        (draw_water && input.model_water).then_some(ModelWaterDepth),
        (draw_water && boats).then_some(RestoreBoatDepth),
        Some(PrepareAlphaDepth),
        (normal && boats).then_some(BoatStencilReduced),
        (normal && boats).then_some(BoatStencilMain),
        (normal && (input.ocean_active || input.water_volume)).then_some(UnderwaterMask),
        Some(PrepareCoarseDepth),
    ]
    .into_iter()
    .flatten()
}

/// 303ae0's colour/foam admission. An enabled ocean with no visible patches
/// still prepares targets and invokes its colour draw; it does not draw foam.
#[derive(Clone, Copy, Debug)]
pub struct WaterColorInputs {
    /// Actual302d20 result, including global disable, owner and enable state.
    pub ocean_active: bool,
    pub ocean_patches: bool,
    pub world_water: bool,
    pub model_water: bool,
    pub world_renderer_present: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaterColorPlan {
    pub prepare_targets: bool,
    pub draw_foam: bool,
    pub draw_ocean: bool,
    pub draw_world: bool,
    pub draw_models: bool,
}

pub fn water_color_plan(input: WaterColorInputs) -> WaterColorPlan {
    WaterColorPlan {
        prepare_targets: input.ocean_active || input.world_water || input.model_water,
        draw_foam: (input.ocean_active && input.ocean_patches)
            || input.world_water
            || input.model_water,
        draw_ocean: input.ocean_active,
        draw_world: input.world_renderer_present && input.world_water,
        draw_models: input.model_water,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refracted_sun_direction_matches_executed_cpu_block() {
        let mut cases = 0;
        for row in include_str!("../../tests/fixtures/water_refracted_light.txt").lines() {
            let w = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(w.len(), 6);
            let actual = refracted_light_direction(std::array::from_fn(|i| f32::from_bits(w[i])));
            for i in 0..3 {
                let expected = f32::from_bits(w[3 + i]);
                if expected.is_nan() {
                    assert!(actual[i].is_nan(), "case{cases} lane{i}");
                } else {
                    assert_eq!(
                        actual[i].to_bits(),
                        expected.to_bits(),
                        "case{cases} lane{i}"
                    );
                }
            }
            cases += 1;
        }
        assert_eq!(cases, 1024);
    }

    #[test]
    fn water_colour_admission_matches_executed_wrapper() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/water_color_plan.txt").lines() {
            let row = line
                .split_whitespace()
                .map(|v| v.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(row.len(), 10);
            let flags = row[2];
            assert_eq!(
                [row[3], row[4]],
                [((row[0] + 3) >> 2).max(2), ((row[1] + 3) >> 2).max(2)]
            );
            let plan = water_color_plan(WaterColorInputs {
                ocean_active: flags & 1 != 0 && flags & 32 == 0 && flags & 64 == 0,
                ocean_patches: flags & 2 != 0,
                world_water: flags & 4 != 0,
                model_water: flags & 8 != 0,
                world_renderer_present: flags & 16 != 0,
            });
            assert_eq!(
                [
                    plan.prepare_targets,
                    plan.draw_foam,
                    plan.draw_ocean,
                    plan.draw_world,
                    plan.draw_models
                ],
                [
                    row[5] != 0,
                    row[6] != 0,
                    row[7] != 0,
                    row[8] != 0,
                    row[9] != 0
                ],
                "flags={flags}"
            );
            count += 1;
        }
        assert_eq!(count, 512);
    }

    #[test]
    fn water_depth_dispatch_matches_executed_task_and_water_wrapper() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/water_depth_sequence.txt").lines() {
            let row: Vec<u8> = line
                .split_whitespace()
                .map(|value| u8::from_str_radix(value, 16).unwrap())
                .collect();
            let flags = row[0];
            let steps = water_depth_steps(WaterDepthInputs {
                alternate_path: flags & 1 != 0,
                ocean_active: flags & 2 != 0,
                water_volume: flags & 4 != 0,
                world_water: flags & 8 != 0,
                model_water: flags & 16 != 0,
                world_boats: flags & 32 != 0,
                model_boats: flags & 64 != 0,
                world_renderer_present: flags & 128 != 0,
            })
            .map(|step| step as u8)
            .collect::<Vec<_>>();
            assert_eq!(&steps, &row[1..], "source dispatch flags {flags:02x}");
            count += 1;
        }
        assert_eq!(count, 256);
    }
}

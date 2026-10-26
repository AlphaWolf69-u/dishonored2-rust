//! 325130's per-surface weighted-transparency parameters. Resolve settings
//! from the rewrite's existing console state; material blend state precedes
//! the accumulation-target overrides applied by the colour submission owner.
use crate::platform::cvars::CVars;

#[derive(Clone, Copy, Debug)]
pub struct TransparencyParameters {
    pub mode: i32,
    pub weights: [f32; 4],
    pub parameters: [f32; 4],
}
impl TransparencyParameters {
    pub fn read(
        cvars: &CVars,
        state: crate::render_state::RenderState,
        particle: bool,
    ) -> Result<Self, String> {
        let mode = cvars.int("r_OIT").ok_or("r_OIT is not registered")?;
        if !(0..=2).contains(&mode) {
            return Err("particle transparency debug resolve is not connected".into());
        }
        let scalar = |name| {
            cvars
                .float(name)
                .ok_or_else(|| format!("{name} is not registered"))
        };
        let source = (state[0] >> 1) & 15;
        let additive = (source == 4 || source == 1) && state[0] & 0x1e0 == 0x20;
        if mode == 0 {
            return Ok(Self {
                mode,
                weights: [0.; 4],
                parameters: [1.; 4],
            });
        }
        let (weights, parameters) = if mode == 2 {
            (
                [
                    scalar(if additive {
                        "OLD_r_oitAdditiveColorResistance"
                    } else {
                        "OLD_r_oitColorResistance"
                    })?,
                    scalar("OLD_r_oitDepthRange")?,
                    scalar(if additive {
                        "OLD_r_oitAdditiveOrderingStrength"
                    } else {
                        "OLD_r_oitOrderingStrength"
                    })?,
                    scalar("OLD_r_oitWeightFactor")?,
                ],
                if additive {
                    [
                        1.,
                        scalar("OLD_r_oitAdditiveAlphaTest")?,
                        scalar("OLD_r_oitRangeAdjustement")?,
                        scalar("OLD_r_oitAdditiveOpacityCoef")?,
                    ]
                } else {
                    [0., 0., scalar("OLD_r_oitRangeAdjustement")?, 1.]
                },
            )
        } else {
            (
                [
                    scalar(if additive {
                        "r_oitAdditiveColorResistance"
                    } else {
                        "r_oitColorResistance"
                    })?,
                    scalar("r_oitDepthRange")?,
                    scalar(if additive {
                        "r_oitAdditiveOrderingStrength"
                    } else {
                        "r_oitOrderingStrength"
                    })?,
                    scalar("r_oitWeightFactor")?,
                ],
                if additive {
                    [
                        1.,
                        scalar("r_oitAdditiveAlphaStrength")?,
                        scalar("r_oitAdditiveParticleColorStrength")?,
                        scalar("r_oitAdditiveOpacity")?,
                    ]
                } else if particle {
                    [
                        1.,
                        scalar("r_oitParticleAlphaStrength")?,
                        scalar("r_oitParticleColorStrength")?,
                        1.,
                    ]
                } else {
                    [1., 0., 1., 1.]
                },
            )
        };
        Ok(Self {
            mode,
            weights,
            parameters,
        })
    }
}

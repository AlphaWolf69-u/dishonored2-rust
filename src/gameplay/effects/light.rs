//!226060/226270/226290: sampled FX light instance. The shared FX pool owns
//!allocation, frame clock, seed and renderer withdrawal, not this pure kernel.
use crate::{assets::fx::light::LightAsset, gameplay::reference_frame::ReferenceFrame};
use super::{envelope::{EnvelopePhase, ScalarEnvelopeSample, VectorEnvelopeSample}, timing::EffectTimings};
use std::sync::Arc;

///2249f0's evaluated inputs. The renderer owner applies3595f0 preset
///photometry and stores these in the same base-light record as authored lights.
#[derive(Clone, Copy, Debug)]
pub struct LightSample {
    pub kind: u32,
    pub preset: u32,
    pub colour: [f32; 3],
    /// decay, falloff, combined-cutoff, specular, scatter-value/origin/X/Y/
    ///offset/turbulence, then spot angular-falloff/combined-range/reflection.
    ///The final three lanes are meaningful only for kind2, as in224ed0.
    pub parameters: [f32; 13],
    pub frame: ReferenceFrame,
    pub culling_offset: [f32; 3],
    pub flags: [u8; 2],
    pub renders_in_fog: bool,
}

pub struct LightEffect {
    asset: Arc<LightAsset>,
    timing: EffectTimings,
    attenuation: [Option<ScalarEnvelopeSample>; 7],
    scattering: [ScalarEnvelopeSample; 6],
    colour: VectorEnvelopeSample,
    offset: VectorEnvelopeSample,
    origin: [f32; 3],
    frame: ReferenceFrame,
    culling_offset: [f32; 3],
    published: bool,
    alive: bool,
}

fn scattering(asset: &LightAsset, seed: &mut u32) -> Result<[ScalarEnvelopeSample; 6], String> {
    //225e20 serialized order; retain each real sample with the instance.
    Ok([
        ScalarEnvelopeSample::new(&asset.scattering[0], seed)?,
        ScalarEnvelopeSample::new(&asset.scattering[1], seed)?,
        ScalarEnvelopeSample::new(&asset.scattering[2], seed)?,
        ScalarEnvelopeSample::new(&asset.scattering[3], seed)?,
        ScalarEnvelopeSample::new(&asset.scattering[4], seed)?,
        ScalarEnvelopeSample::new(&asset.scattering[5], seed)?,
    ])
}
fn attenuation(asset: &LightAsset, seed: &mut u32) -> Result<[Option<ScalarEnvelopeSample>; 7], String> {
    let mut values = std::array::from_fn(|_| None);
    //225b80 does not consume angular samples for non-spot lights.
    for index in [0, 1, 2, 6] {
        values[index] = Some(ScalarEnvelopeSample::new(&asset.attenuation[index], seed)?);
    }
    if asset.kind == 2 {
        for index in [3, 4, 5] {
            values[index] = Some(ScalarEnvelopeSample::new(&asset.attenuation[index], seed)?);
        }
    }
    Ok(values)
}

impl LightEffect {
    pub fn new(asset: Arc<LightAsset>, offsets: [f32; 4], owner: ReferenceFrame,
        seed: &mut u32) -> Result<Self, String> {
        //226060: timing, attenuation, scattering, colour, then position.
        let timing = EffectTimings::new(&asset.timing, offsets, seed);
        let attenuation = attenuation(&asset, seed)?;
        let scattering = scattering(&asset, seed)?;
        let colour = VectorEnvelopeSample::new(&asset.colour, seed)?;
        let offset = VectorEnvelopeSample::new(&asset.position.offset, seed)?;
        Ok(Self { asset, timing, attenuation, scattering, colour, offset,
            origin: owner.position, frame: owner, culling_offset: [0.; 3],
            published: false, alive: true })
    }

    ///224e80/224e90: normal Stop touches all entries; stop-looping only
    ///touches entries whose real descriptor is looping and still alive.
    pub fn is_alive(&self) -> bool { self.alive }
    pub fn can_loop(&self) -> bool { self.alive && self.asset.looping }

    pub fn stop(&mut self, now: f32) {
        self.timing.begin_fade_out(self.asset.finish_loop_on_stop, now);
    }

    ///2248c0 selects the exact envelope phase. None means no publication:
    ///delay and the terminal update leave the existing render record alone.
    ///The pool checks is_alive BEFORE update and withdraws only next traversal.
    pub fn update(&mut self, now: f32, owner: ReferenceFrame, seed: &mut u32)
        -> Result<Option<LightSample>, String> {
        if !self.alive || !(self.timing.delay_end <= now) { return Ok(None); }
        let (phase, time) = if now >= self.timing.duration_end {
            if !self.asset.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                let time = (now - self.timing.duration_end) * self.timing.fade_out_rate;
                if !(time <= 1.) { self.alive = false; return Ok(None); }
                (EnvelopePhase::FadeOut, time)
            } else {
                self.timing.restart(&self.asset.timing, self.timing.duration_end, seed);
                self.resample_loop(seed)?;
                if now >= self.timing.fade_in_end {
                    if now >= self.timing.duration_end { return Ok(None); }
                    (EnvelopePhase::Duration, (now-self.timing.fade_in_end)*self.timing.duration_rate)
                } else {
                    (EnvelopePhase::FadeIn, (now-self.timing.delay_end)*self.timing.fade_in_rate)
                }
            }
        } else if now >= self.timing.fade_in_end {
            (EnvelopePhase::Duration, (now-self.timing.fade_in_end)*self.timing.duration_rate)
        } else {
            (EnvelopePhase::FadeIn, (now-self.timing.delay_end)*self.timing.fade_in_rate)
        };
        self.sample(phase, time, owner).map(Some)
    }

    fn resample_loop(&mut self, seed: &mut u32) -> Result<(), String> {
        //225a90's loop ordering differs from Start: position precedes colour.
        if self.asset.randomize_attenuation { self.attenuation = attenuation(&self.asset, seed)?; }
        if self.asset.randomize_scattering { self.scattering = scattering(&self.asset, seed)?; }
        if self.asset.position.randomize_each_loop {
            self.offset = VectorEnvelopeSample::new(&self.asset.position.offset, seed)?;
        }
        if self.asset.randomize_colour { self.colour = VectorEnvelopeSample::new(&self.asset.colour, seed)?; }
        Ok(())
    }

    fn sample(&mut self, phase: EnvelopePhase, time: f32, owner: ReferenceFrame)
        -> Result<LightSample, String> {
        let mut attenuation = [0.; 7];
        //224ed0/2252e0/2256d0 preserve raw current values for the next phase;
        //clamping below changes only the renderer inputs, not those samples.
        for index in [0, 1, 2, 6] {
            attenuation[index] = self.attenuation[index].as_mut().ok_or("FX light scalar sample absent")?
                .sample(&self.asset.attenuation[index], phase, time)?;
        }
        if self.asset.kind == 2 {
            for index in [3, 4, 5] {
                attenuation[index] = self.attenuation[index].as_mut().ok_or("FX spot scalar sample absent")?
                    .sample(&self.asset.attenuation[index], phase, time)?;
            }
        }
        let mut parameters = [0.; 13];
        //DAT141ab3810; source comparison preserves NaNs, unlike f32::max.
        let minimum = |value: f32| if value < 0.01 { 0.01 } else { value };
        parameters[0] = minimum(attenuation[0]);
        parameters[1] = minimum(attenuation[1]);
        parameters[2] = parameters[1] + minimum(attenuation[2]);
        parameters[3] = attenuation[6];
        for index in 0..6 {
            parameters[4+index] = self.scattering[index].sample(&self.asset.scattering[index], phase, time)?;
        }
        if self.asset.kind == 2 {
            parameters[10] = minimum(attenuation[3]);
            parameters[11] = parameters[10] + minimum(attenuation[4]);
            parameters[12] = attenuation[5];
        }
        let colour = self.colour.sample(&self.asset.colour, phase, time)?;
        let offset = self.offset.sample(&self.asset.position.offset, phase, time)?;
        self.publish_frame(owner, offset);
        //232780 initializes0x13.226060 replaces cast-shadow/scatter bits,
        //but preserves constructor bit4;2249f0 sets the active bit again.
        let flags = [0x11 | (u8::from(self.asset.cast_shadows)<<1)
                | (u8::from(self.asset.volumetric_shadows)<<5)
                | (u8::from(self.asset.preserve_volumetric_shadow)<<6)
                | (u8::from(self.asset.second_turbulence)<<7),
            u8::from(self.asset.scatter_only) | (u8::from(self.asset.custom_scatter_shape)<<1)];
        Ok(LightSample { kind: self.asset.kind, preset: self.asset.preset,
            colour: std::array::from_fn(|i| colour[i] * self.asset.base_colour[i]),
            parameters, frame: self.frame, culling_offset: self.culling_offset,
            flags, renders_in_fog: self.asset.renders_in_fog })
    }

    fn publish_frame(&mut self, owner: ReferenceFrame, offset: [f32; 3]) {
        //2249f0 caches the Start translation when not tracking position, but
        //still rotates each current local offset by the current owner basis.
        if self.asset.position.track_position { self.origin = owner.position; }
        let rotate = |value: [f32; 3]| std::array::from_fn(|i|
            value[0]*owner.basis[0][i] + value[1]*owner.basis[1][i] + value[2]*owner.basis[2][i]);
        self.frame.position = if self.asset.position.worldspace {
            std::array::from_fn(|i| self.origin[i] + offset[i])
        } else {
            std::array::from_fn(|i| ((self.origin[i] + offset[0]*owner.basis[0][i])
                + offset[1]*owner.basis[1][i]) + offset[2]*owner.basis[2][i])
        };
        self.culling_offset = if self.asset.position.worldspace {
            self.asset.position.culling_offset
        } else { rotate(self.asset.position.culling_offset) };
        if !self.published || self.asset.position.track_orientation { self.frame.basis = owner.basis; }
        self.published = true;
    }
}

//! Host stereo block routing around the recovered bus DSP. Logical buses
//! fold into connections; mixing buses own shared effects and output ramps
//! (14111f200, 14112ba90, 14112c660, 1410eda30).
use crate::gameplay::sound::{bus_effects::{BusEffects, EffectTables}, gain::GainRamp,
    mix::{power10, Mixer}, tail::{BufferState, BufferStatus}};
use std::{collections::HashMap, path::Path};

pub(in crate::app::audio) const FRAMES: usize = 1024; // 142427c30; also the source resampling quantum.

struct Bus {
    id: u32,
    parent: Option<usize>,
    channels: [Vec<f32>; 2],
    effects: BusEffects,
    gain: GainRamp,
    target: f32,
    input: bool,
    started: bool,
}

pub(in crate::app::audio) struct Buses {
    nodes: Vec<Bus>,
    indices: HashMap<u32, usize>,
    primary: u32,
    output: [f32; FRAMES * 2],
}

impl Buses {
    pub fn new(mixer: &Mixer, base: &Path) -> Result<Self, String> {
        let tables = EffectTables::load(mixer, base)?;
        let mut result = Self { nodes: Vec::new(), indices: HashMap::new(),
            primary: mixer.primary_bus.ok_or("Wwise initialization bank has no primary bus")?,
            output: [0.0; FRAMES * 2] };
        for &id in &mixer.bus_order {
            if mixer.bus_mixes(id)? { result.insert(id, mixer, &tables)?; }
        }
        Ok(result)
    }

    fn insert(&mut self, id: u32, mixer: &Mixer, tables: &EffectTables) -> Result<usize, String> {
        if let Some(&index) = self.indices.get(&id) { return Ok(index); }
        let definition = mixer.buses.get(&id).ok_or("missing mixing bus definition")?;
        let settings = definition.bus_settings.as_ref().ok_or("mixing bus has no settings")?;
        let parent_id = mixer.mixing_bus(definition.parent)?;
        let parent = if parent_id == 0 { None } else { Some(self.insert(parent_id, mixer, tables)?) };
        if settings.flags & 1 != 0 { return Err(format!("bus {id:#010x} requires bus positioning")); }
        if settings.hdr_flags & 1 != 0 { return Err(format!("bus {id:#010x} requires HDR gain processing")); }
        if settings.mixer_id != 0 { return Err(format!("bus {id:#010x} requires its mixer plugin")); }
        if settings.channel_config != 0 && settings.channel_config != 0x3102 {
            return Err(format!("bus {id:#010x} requires channel conversion from its authored format"));
        }
        let effects = BusEffects::new(definition, mixer, tables, 48000, 3, settings.auxiliary)?;
        let index = self.nodes.len();
        self.nodes.push(Bus { id, parent, channels: [vec![0.0; FRAMES], vec![0.0; FRAMES]], effects,
            gain: GainRamp::default(), target: power10(mixer.bus_output_db(id)? * 0.05), input: false, started: false });
        self.indices.insert(id, index);
        Ok(index)
    }

    /// Settings are evaluated by the host; the processing callback never
    /// needs an asset reader or a console lock.
    pub fn update(&mut self, mixer: &Mixer) -> Result<(), String> {
        for bus in &mut self.nodes {
            bus.target = power10(mixer.bus_output_db(bus.id)? * 0.05);
            bus.effects.set_bypass(mixer.bus_bypass(mixer.buses.get(&bus.id).ok_or("mixing bus disappeared")?, &[])?);
        }
        Ok(())
    }

    pub fn begin(&mut self) {
        self.output.fill(0.0);
        for bus in &mut self.nodes {
            bus.input = false;
            for channel in &mut bus.channels { channel.fill(0.0); }
        }
    }

    pub fn add(&mut self, id: u32, frame: usize, sample: [f32; 2]) -> Result<(), String> {
        if frame >= FRAMES { return Err("audio bus frame exceeds its processing quantum".into()); }
        if id == 0 {
            self.output[frame * 2] += sample[0];
            self.output[frame * 2 + 1] += sample[1];
            return Ok(());
        }
        let bus = &mut self.nodes[*self.indices.get(&id).ok_or_else(|| format!("unresolved mixing bus {id:#010x}"))?];
        bus.input = true;
        for (channel, sample) in bus.channels.iter_mut().zip(sample) { channel[frame] += sample; }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<&[f32; FRAMES * 2], String> {
        // Parent slots are created before children. Reverse order delivers
        // every child's block before its parent's effects run.
        for index in (0..self.nodes.len()).rev() {
            let (parents, current) = self.nodes.split_at_mut(index);
            let bus = &mut current[0];
            if !bus.started && !bus.input { continue; }
            if !bus.started {
                bus.effects.reset();
                bus.gain = GainRamp::default();
            }
            let status = BufferStatus { state: if bus.input { BufferState::DataReady } else { BufferState::NoMoreData },
                valid_frames: if bus.input { FRAMES as u16 } else { 0 } };
            let [left, right] = &mut bus.channels;
            let status = bus.effects.process_buffer(&mut [left.as_mut_slice(), right.as_mut_slice()], status)?;
            let count = usize::from(status.valid_frames);
            bus.started = count != 0;
            if count == 0 { continue; }
            bus.gain.begin([bus.target; 8], FRAMES as u16);
            if let Some(parent) = bus.parent {
                let parent = &mut parents[parent];
                parent.input = true;
                for frame in 0..count {
                    let gain = bus.gain.next_frame()[0];
                    for (output, input) in parent.channels.iter_mut().zip(&bus.channels) { output[frame] += input[frame] * gain; }
                }
            } else if bus.id == self.primary {
                for frame in 0..count {
                    let gain = bus.gain.next_frame()[0];
                    self.output[frame * 2] += bus.channels[0][frame] * gain;
                    self.output[frame * 2 + 1] += bus.channels[1][frame] * gain;
                }
            }
        }
        Ok(&self.output)
    }
}

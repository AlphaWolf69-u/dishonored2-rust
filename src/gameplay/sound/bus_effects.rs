//! Host ownership of the installed bus effect chains. Each bus owns one
//! processor chain; all voices routed to that bus share its histories.
use super::{matrix_reverb, mix::Mixer, room_reverb, tail::BufferStatus};
use crate::{assets::{sound_effects::{matrix_delays, ReflectionPatterns}, wwise::NodeProperties}};
use std::{collections::{BTreeMap, BTreeSet}, path::Path, sync::Arc};

pub struct EffectTables {
    reflections: Arc<ReflectionPatterns>,
    matrix_delays: Option<Arc<[f32; 16]>>,
}
impl EffectTables {
    /// Resolve installed executable data before submitting any DSP to the
    /// audio worker. No file access belongs in a processing callback.
    pub fn load(mixer: &Mixer, game_base: &Path) -> Result<Self, String> {
        let mut indices = BTreeSet::new();
        let mut matrix = false;
        for definition in mixer.effects.values() {
            match definition.plugin {
                room_reverb::PLUGIN => {
                    let parameters = room_reverb::Parameters::read(&definition.parameters)?;
                    if parameters.early_reflections { indices.insert(parameters.early_pattern); }
                },
                matrix_reverb::PLUGIN => matrix = true,
                _ => {},
            }
        }
        Ok(Self { reflections: Arc::new(ReflectionPatterns::load(game_base, indices)?),
            matrix_delays: if matrix { Some(Arc::new(matrix_delays(game_base)?)) } else { None } })
    }
}

enum Processor {
    Room(room_reverb::RoomReverb),
    Matrix(matrix_reverb::MatrixReverb),
}
pub struct BusEffects {
    slots: Vec<(u8, Processor)>,
}
impl BusEffects {
    pub fn new(bus: &NodeProperties, mixer: &Mixer, tables: &EffectTables,
        rate: u32, mask: u32, send: bool) -> Result<Self, String>
    {
        if bus.effects.bypass != 0 {
            return Err(format!("bus {:#010x} requires effect bypass transitions", bus.id));
        }
        // 141114910 assigns one definition to each of four slots. A later
        // serialized assignment replaces an earlier assignment to that slot.
        let mut references = BTreeMap::new();
        for reference in &bus.effects.slots {
            if reference.id == 0 { continue; } // 1411204d0 skips empty entries.
            if reference.slot >= 4 { return Err("bus effect slot exceeds four native slots".into()); }
            references.insert(reference.slot, reference);
        }
        let mut slots = Vec::new();
        for (slot, reference) in references {
            let definition = mixer.effects.get(&(reference.shared, reference.id))
                .ok_or_else(|| format!("missing bus effect definition {:#010x}", reference.id))?;
            if !definition.curves.is_empty() || !definition.values.is_empty() {
                return Err(format!("bus effect {:#010x} requires live parameter subscriptions", definition.id));
            }
            if !definition.media.is_empty() {
                return Err(format!("bus effect {:#010x} requires plugin media bindings", definition.id));
            }
            let processor = match definition.plugin {
                room_reverb::PLUGIN => Processor::Room(room_reverb::RoomReverb::new(
                    room_reverb::Parameters::read(&definition.parameters)?, tables.reflections.clone(), rate, mask, send)?),
                matrix_reverb::PLUGIN => Processor::Matrix(matrix_reverb::MatrixReverb::new(
                    matrix_reverb::Parameters::read(&definition.parameters)?,
                    tables.matrix_delays.clone().ok_or("matrix reverb table was not loaded")?, rate, mask, send)?),
                plugin => return Err(format!("bus effect plugin {plugin:#010x} has no processing implementation")),
            };
            slots.push((slot, processor));
        }
        Ok(Self { slots })
    }

    pub fn set_parameter(&mut self, slot: u8, id: u16, bytes: &[u8]) -> Result<(), String> {
        let (_, processor) = self.slots.iter_mut().find(|(index, _)| *index == slot)
            .ok_or("bus effect parameter targets an empty slot")?;
        match processor {
            Processor::Room(effect) => effect.set_parameter(id, bytes),
            Processor::Matrix(effect) => effect.set_parameter(id, bytes),
        }
    }

    pub fn reset(&mut self) {
        for (_, processor) in &mut self.slots {
            match processor { Processor::Room(effect) => effect.reset(), Processor::Matrix(effect) => effect.reset() }
        }
    }

    pub fn process_buffer(&mut self, channels: &mut [&mut [f32]], mut status: BufferStatus) -> Result<BufferStatus, String> {
        for (_, processor) in &mut self.slots {
            status = match processor {
                Processor::Room(effect) => effect.process_buffer(channels, status)?,
                Processor::Matrix(effect) => effect.process_buffer(channels, status)?,
            };
        }
        Ok(status)
    }
}

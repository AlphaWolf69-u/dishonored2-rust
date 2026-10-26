//! Group2 physics API packets and the game's4096-byte deferred command owner.
use super::{command_buffer::CommandBuffer, BodyAddition, BodyPublication, TransformCallbacks};
use crate::gameplay::{
    body_storage::BodyActivation, shape_transform::ShapeMatrix, space_cells::SpaceCells,
};

#[derive(Clone, Copy, Debug)]
pub enum BodyCommand {
    Add {
        body: u32,
        mode: u8,
        activation: u8,
    },
    Transform {
        body: u32,
        frame: ShapeMatrix,
        mode: u8,
    },
    Velocity {
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    },
    Activation {
        body: u32,
        deactivate: bool,
    },
    Ignored,
}

impl BodyCommand {
    /// 089fb0 switches on the u16 at+4. Explicit source no-op codes stay no-op;
    /// understood routing with still-unported callees returns an honest error.
    pub fn decode(group: u8, packet: &[u8]) -> Result<Self, String> {
        if group != 2 {
            return Err(format!(
                "physics command dispatcher group{group} is not implemented"
            ));
        }
        let u16_at = |at: usize| {
            packet
                .get(at..at + 2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .ok_or("truncated physics command header")
        };
        let opcode = u16_at(4)?;
        let size = u16_at(0)? as usize;
        let need = match opcode {
            1 | 0x1e => 16,
            6 => 80,
            9 => 48,
            _ => 0,
        };
        if need != 0 && (size < need || packet.len() < need) {
            return Err("physics command payload is truncated".into());
        }
        let word = |at: usize| u32::from_le_bytes(packet[at..at + 4].try_into().unwrap());
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(word(at + i * 4)));
        Ok(match opcode {
            1 => Self::Add {
                body: word(8),
                mode: packet[12],
                activation: packet[13],
            },
            6 => Self::Transform {
                body: word(8),
                frame: std::array::from_fn(|i| vector(16 + i * 16)),
                mode: packet[12],
            },
            9 => Self::Velocity {
                body: word(8),
                linear: vector(16),
                angular: vector(32),
            },
            0x1e => Self::Activation {
                body: word(8),
                deactivate: packet[12] != 0,
            },
            1..=0x23 | 0x27..=0x34 | 0x36 | 0x39..=0x3e => {
                return Err(format!(
                    "physics API command0x{opcode:x} is not implemented"
                ))
            }
            _ => Self::Ignored,
        })
    }

    pub fn apply(
        self,
        world: &mut BodyPublication<'_>,
        cells: &SpaceCells,
        callbacks: &mut impl TransformCallbacks,
    ) -> Result<(), String> {
        match self {
            Self::Add {
                body,
                mode,
                activation,
            } => {
                let mode = match mode {
                    0 => BodyAddition::Disabled,
                    1 => BodyAddition::Immediate,
                    2 => BodyAddition::Queued,
                    _ => return Err("body-add command has an invalid admission mode".into()),
                };
                let activation = match activation {
                    0 => BodyActivation::Activate,
                    1 => BodyActivation::KeepDeactivated,
                    _ => return Err("body-add command has an invalid activation mode".into()),
                };
                world.add(&[body], mode, activation, callbacks)?;
            }
            Self::Transform { body, frame, mode } => {
                world.set_transform(body, frame, i32::from(mode), cells, callbacks)?;
            }
            Self::Velocity {
                body,
                linear,
                angular,
            } => {
                world.set_velocity(body, linear, angular, 0, callbacks)?;
            }
            Self::Activation { body, deactivate } => {
                world.set_activation(body, deactivate, callbacks)?;
            }
            Self::Ignored => {}
        }
        Ok(())
    }
}

pub struct PhysicsCommands {
    buffer: CommandBuffer,
}
impl PhysicsCommands {
    /// 08f940 initializes world-owner+148288 with4096 bytes and memory tag95.
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            buffer: CommandBuffer::new(0x1000, 0x5f)?,
        })
    }

    pub fn enqueue_transform(
        &self,
        body: u32,
        frame: ShapeMatrix,
        mode: u8,
        grow_one_at_a_time: bool,
    ) -> Result<(), String> {
        let mut packet = [0u8; 80];
        packet[..2].copy_from_slice(&80u16.to_le_bytes());
        packet[2..6].copy_from_slice(&0x60200u32.to_le_bytes());
        packet[8..12].copy_from_slice(&body.to_le_bytes());
        packet[12] = mode;
        for (i, value) in frame.into_iter().flatten().enumerate() {
            packet[16 + i * 4..20 + i * 4].copy_from_slice(&value.to_bits().to_le_bytes());
        }
        self.buffer.append_packet(&packet, grow_one_at_a_time)?;
        Ok(())
    }

    pub fn enqueue_velocity(
        &self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
        grow_one_at_a_time: bool,
    ) -> Result<(), String> {
        let mut packet = [0u8; 48];
        packet[..2].copy_from_slice(&48u16.to_le_bytes());
        packet[2..6].copy_from_slice(&0x90200u32.to_le_bytes());
        packet[8..12].copy_from_slice(&body.to_le_bytes());
        for (i, value) in linear.into_iter().chain(angular).enumerate() {
            packet[16 + i * 4..20 + i * 4].copy_from_slice(&value.to_bits().to_le_bytes());
        }
        self.buffer.append_packet(&packet, grow_one_at_a_time)?;
        Ok(())
    }

    pub fn dispatch(
        &mut self,
        world: &mut BodyPublication<'_>,
        cells: &SpaceCells,
        callbacks: &mut impl TransformCallbacks,
    ) -> Result<(), String> {
        self.buffer.dispatch(|group, packet| {
            BodyCommand::decode(group, packet)?.apply(world, cells, callbacks)
        })
    }

    pub fn reset(&mut self, release: bool) -> Result<(), String> {
        self.buffer.reset(release)
    }
}

#[cfg(test)]
mod tests;

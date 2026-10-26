//! 2184b0/218620/2189d0: connect sampled FX placement to the decal manager.
use super::{DecalDescription, DecalPool};
use crate::{
    float::{add, multiply, subtract},
    gameplay::reference_frame::{basis_product, ReferenceFrame},
};
use std::num::NonZeroU64;

/// The FX owner supplies its sampled local frame, authored pivot and follow
/// switches. Parent frame selection and curve sampling remain with that owner.
pub struct DecalAttachment {
    pub local: ReferenceFrame,
    pub pivot: [f32; 3],
    pub follow_position: bool,
    pub follow_orientation: bool,
}
impl DecalAttachment {
    ///2184b0 offsets the asymmetric projection box before composing its parent.
    pub fn world_position(&self, scale: [f32; 3], parent: ReferenceFrame) -> [f32; 3] {
        let anchor = [
            multiply(multiply(subtract(0.5, self.pivot[0]), 2.), scale[0]),
            multiply(multiply(subtract(0.5, self.pivot[1]), 2.), scale[1]),
            multiply(subtract(1., self.pivot[2]), scale[2]),
        ];
        let transform = |frame: ReferenceFrame, position: [f32; 3]| {
            std::array::from_fn(|i| {
                add(
                    add(
                        add(
                            multiply(position[1], frame.basis[1][i]),
                            multiply(position[0], frame.basis[0][i]),
                        ),
                        multiply(position[2], frame.basis[2][i]),
                    ),
                    frame.position[i],
                )
            })
        };
        transform(parent, transform(self.local, anchor))
    }
}

pub struct DecalEffectBinding {
    pub active: bool,
    slot: Option<u32>,
}
impl DecalEffectBinding {
    /// Activation is supplied by the FX scheduler, not inferred from geometry.
    pub fn new(active: bool) -> Self {
        Self { active, slot: None }
    }
    pub fn slot(&self) -> Option<u32> {
        self.slot
    }

    ///2189d0's eviction notification does not free the replacement pool slot.
    pub fn retire(&mut self) {
        self.active = false;
        self.slot = None;
    }

    ///218620 resolves the material once when creating a record. Following
    ///position/orientation are independent; scale and colour update every call.
    ///The caller routes evictions back to the previous owner's retire method.
    pub fn apply<M, R>(
        &mut self,
        owner: NonZeroU64,
        pool: &mut DecalPool<DecalDescription<M>, R>,
        attachment: &DecalAttachment,
        parent: ReferenceFrame,
        scale: [f32; 3],
        colour: [f32; 4],
        load_material: impl FnOnce() -> Result<M, String>,
        evict: impl FnMut(NonZeroU64),
    ) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        if self.slot.is_none() {
            let description = DecalDescription {
                position: attachment.world_position(scale, parent),
                axes: basis_product(attachment.local.basis, parent.basis),
                scale,
                colour,
                material: load_material()?,
            };
            self.slot = Some(pool.insert(owner, description, evict)?);
        } else {
            //1c0450 finds the actual owner; its saved index argument is unused.
            let index = pool
                .find(owner)?
                .ok_or("active decal FX lost its pool owner")?;
            let description = pool
                .slot_mut(index)
                .unwrap()
                .description
                .as_mut()
                .ok_or("active decal FX has no pool description")?;
            if attachment.follow_position {
                description.position = attachment.world_position(scale, parent);
            }
            if attachment.follow_orientation {
                description.axes = basis_product(attachment.local.basis, parent.basis);
            }
            description.scale = scale;
            description.colour = colour;
        }
        Ok(())
    }
}

//! 2184b0/218620/2189d0: connect sampled FX placement to the decal manager.
use super::{DecalDescription, DecalPool};
use crate::{
    float::{add, multiply, subtract},
    gameplay::{
        effects::{
            decal::{DecalAnimation, DecalAnimationDefinition, DecalPhaseSample},
            parameters::position_range,
        },
        reference_frame::{basis_product, normal_axis_basis, ReferenceFrame},
    },
};
use std::num::NonZeroU64;
use std::sync::Arc;

pub struct DecalAttachmentDefinition {
    pub projection_axis: i32,
    pub initial_rotation: [f32; 2],
    pub initial_position: [[f32; 3]; 2],
    pub centering: [f32; 3],
    pub follow_position: bool,
    pub follow_orientation: bool,
}
impl Default for DecalAttachmentDefinition {
    ///217d50 resets following, Z projection, centred XY/front-face Z and zero
    ///local rotation/translation. It does not guess a world placement.
    fn default() -> Self {
        Self {
            projection_axis: 2,
            initial_rotation: [0.; 2],
            initial_position: [[0.; 3]; 2],
            centering: [0.5, 0.5, 1.],
            follow_position: true,
            follow_orientation: true,
        }
    }
}
impl DecalAttachmentDefinition {
    ///The attachment portion of218e80 consumes rotation, then X/Y/Z position.
    pub fn sample(&self, seed: &mut u32) -> DecalAttachment {
        let t = crate::random::lcg_unit(seed);
        let angle = multiply(
            add(
                multiply(
                    subtract(1., t),
                    crate::gameplay::effects::normalize_small(self.initial_rotation[1]),
                ),
                multiply(
                    t,
                    crate::gameplay::effects::normalize_small(self.initial_rotation[0]),
                ),
            ),
            f32::from_bits(0x3c8e_fa35),
        );
        let rotation = super::super::render_shadow::axis_rotation(2, angle).unwrap();
        let basis = basis_product(rotation, normal_axis_basis(self.projection_axis));
        let mix = std::array::from_fn(|_| crate::random::lcg_unit(seed));
        DecalAttachment {
            local: ReferenceFrame {
                basis,
                position: position_range(self.initial_position, mix),
            },
            pivot: self.centering,
            follow_position: self.follow_position,
            follow_orientation: self.follow_orientation,
        }
    }
}

///Retained FX state. Definition data is shared; only sampled parameters,
///attachment and manager binding belong to each running instance.
pub struct DecalEffect {
    pub animation: DecalAnimation,
    pub attachment: DecalAttachment,
    pub binding: DecalEffectBinding,
}
impl DecalEffect {
    ///218e80 resolves no material or GPU object here. Those are created lazily
    ///by218620 on the first active phase update.
    pub fn new(
        definition: Arc<DecalAnimationDefinition>,
        attachment: &DecalAttachmentDefinition,
        offsets: [f32; 4],
        seed: &mut u32,
    ) -> Result<Self, String> {
        let (animation, attachment) =
            DecalAnimation::new(
                definition,
                offsets,
                seed,
                |seed| Ok(attachment.sample(seed)),
            )?;
        Ok(Self {
            animation,
            attachment,
            binding: DecalEffectBinding::new(true),
        })
    }
}

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

    ///2191f0 updates only active FX; a pooled-but-retired entry cannot consume
    ///loop RNG or modify its retained animation state before group removal.
    pub fn update(
        &self,
        animation: &mut DecalAnimation,
        now: f32,
        seed: &mut u32,
    ) -> Result<Option<DecalPhaseSample>, String> {
        if self.active {
            animation.update(now, seed)
        } else {
            Ok(None)
        }
    }

    ///218980 removes the cached record during group retirement. Unlike an
    ///eviction notification, this leaves the retired FX's cached fields intact
    ///until initialization reuses it. Stale bindings cannot remove a new owner.
    pub fn detach<M, R>(
        &self,
        owner: NonZeroU64,
        pool: &mut DecalPool<DecalDescription<M>, R>,
        unregister: impl FnMut(&mut R),
    ) -> Result<bool, String> {
        match self.slot {
            Some(index) => pool.remove(index, owner, unregister),
            None => Ok(false),
        }
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
        sample: DecalPhaseSample,
        load_material: impl FnOnce() -> Result<M, String>,
        evict: impl FnMut(NonZeroU64),
    ) -> Result<(), String> {
        //218d10 expires the FX before218620's activation check. The old pool
        //record remains until the scheduler retires it; do not apply another
        //fade sample or remove a slot that might already belong to a new FX.
        if sample.expired {
            self.active = false;
        }
        if !self.active {
            return Ok(());
        }
        let scale = sample.value.scale;
        let colour = sample.value.colour;
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

//! 33bf30's projected WORLD draw packets and 29a270's shared 64-byte payloads.
use super::{CommittedDecal, DecalDescription, DecalPool};
use crate::assets::decal::DecalMaterials;
use std::ops::Range;

/// MAIN presence is resolved for the actual instancing bank by the material
/// owner. DEPTH_BOX has its own draw path and does not enter this mesh list.
#[derive(Clone)]
pub struct ProjectionMaterial<M> {
    pub material: M,
    pub main_present: bool,
    pub depth_box: bool,
}
impl<M> ProjectionMaterial<M> {
    fn admitted(&self) -> bool {
        self.main_present && !self.depth_box
    }
}

/// The WORLD packet's contiguous instance range and 2d73b0's classification.
pub struct WorldReceiver {
    pub instances: Range<u32>,
    pub main_blend_state: u32,
    pub flags_48: u8,
}

#[repr(C)]
pub struct ProjectionData {
    pub world_to_decal: [[f32; 4]; 3],
    pub colour: [f32; 4],
}
pub struct ProjectedWorldDraw<'a, M> {
    pub receiver: usize,
    pub material: &'a M,
    /// High byte selects ProjectionData; low 24 bits retain the WORLD ID.
    pub instances: Vec<u32>,
}
pub struct ProjectedWorldDraws<'a, M> {
    pub projections: Vec<ProjectionData>,
    pub draws: Vec<ProjectedWorldDraw<'a, M>>,
}

/// 298ee0's retained target cursor skips every range whose selected MAIN is
/// absent or depth-box-only. Ranges are ordered and contiguous: the last end
/// in each receiver class is sufficient for the same prepass.
pub fn receiver_target_floor(last_ends: [u32; 2], admitted: [bool; 2]) -> u32 {
    (0..2)
        .filter(|&class| !admitted[class])
        .map(|class| last_ends[class])
        .max()
        .unwrap_or(0)
}

pub fn receiver_accepts_geometry(depth_box: bool, main_blend_state: u32, flags_48: u8) -> bool {
    !depth_box || (main_blend_state & 1 != 0 && flags_48 & 0x30 == 0)
}

impl<C: Clone> DecalPool<DecalDescription<C>, CommittedDecal<C>> {
    /// Consume the same physical-slot order as prepare_world. A live slot must
    /// have completed projection preparation before it can enter the draw list.
    pub fn world_draws<M>(
        &self,
        receivers: &[WorldReceiver],
        visible_instances: &[u8],
    ) -> Result<ProjectedWorldDraws<'_, M>, String>
    where
        C: AsRef<DecalMaterials<ProjectionMaterial<M>>>,
    {
        let decals = (0..self.limit() as u32)
            .filter_map(|index| {
                let slot = self.slot(index).unwrap();
                slot.owner().is_some().then(|| {
                    slot.renderer
                        .as_ref()
                        .ok_or_else(|| format!("decal slot {index} has no committed projection"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        gather_world_draws(&decals, receivers, visible_instances)
    }
}

/// Keep receiver order, projection order, actual instance visibility and target
/// cursors. 298ee0's projection-bounds visibility bit controls material updates,
/// not this WORLD draw loop; it must not introduce another geometry cull here.
pub fn gather_world_draws<'a, M: 'a, C: AsRef<DecalMaterials<ProjectionMaterial<M>>>>(
    decals: &[&'a CommittedDecal<C>],
    receivers: &[WorldReceiver],
    visible_instances: &[u8],
) -> Result<ProjectedWorldDraws<'a, M>, String> {
    let mut previous_end = 0;
    let mut last_ends = [0; 2];
    for receiver in receivers {
        if receiver.instances.start != previous_end
            || receiver.instances.end < receiver.instances.start
            || receiver.instances.end as usize > visible_instances.len()
            || receiver.instances.end > 0x0100_0000
        {
            return Err("projected WORLD receiver ranges are invalid".into());
        }
        previous_end = receiver.instances.end;
        last_ends[usize::from(receiver.main_blend_state & 1 != 0)] = receiver.instances.end;
    }
    let mut cursors = vec![0; decals.len()];
    let mut indices = vec![None; decals.len()];
    for (index, decal) in decals.iter().enumerate() {
        let targets = &decal.projection.targets;
        if targets.windows(2).any(|pair| pair[0] >= pair[1])
            || targets
                .last()
                .is_some_and(|&i| i as usize >= visible_instances.len())
        {
            return Err("projected WORLD target IDs are invalid".into());
        }
        let admitted = [true, false].map(|opaque| {
            decal
                .description
                .material
                .as_ref()
                .select(opaque, true)
                .is_some_and(|material| material.admitted())
        });
        let floor = receiver_target_floor(last_ends, admitted);
        cursors[index] = targets.partition_point(|&target| target < floor);
    }
    let mut output = ProjectedWorldDraws {
        projections: Vec::new(),
        draws: Vec::new(),
    };
    for (receiver_index, receiver) in receivers.iter().enumerate() {
        // The source stops the WORLD packet loop once its 8-bit payload
        // domain is full, including previously assigned projections.
        if output.projections.len() == 256 {
            break;
        }
        for (index, decal) in decals.iter().enumerate() {
            if decal.projection.targets.is_empty() {
                continue;
            }
            let Some(material) = decal
                .description
                .material
                .as_ref()
                .select(receiver.main_blend_state & 1 == 0, true)
                .filter(|m| m.admitted())
            else {
                continue;
            };
            let mut instances = Vec::new();
            let targets = &decal.projection.targets;
            // 298ee0's depth-box target producer retains only nonopaque
            // receivers without decal/bit5 exclusions. Opaque receivers use
            // its separate dynamic-receiver registration path.
            let depth_box_targets = decal
                .description
                .material
                .as_ref()
                .opaque_instanced
                .as_ref()
                .is_some_and(|m| m.depth_box);
            let receiver_allowed = receiver_accepts_geometry(
                depth_box_targets,
                receiver.main_blend_state,
                receiver.flags_48,
            );
            while let Some(&target) = targets.get(cursors[index]) {
                if target >= receiver.instances.end {
                    break;
                }
                if receiver_allowed && visible_instances[target as usize] != 0 {
                    let projection = *indices[index].get_or_insert_with(|| {
                        let payload = output.projections.len() as u32;
                        output.projections.push(ProjectionData {
                            world_to_decal: std::array::from_fn(|row| decal.world_to_decal[row]),
                            colour: decal.description.colour,
                        });
                        payload
                    });
                    instances.push((projection << 24) | target);
                }
                cursors[index] += 1;
            }
            if !instances.is_empty() {
                output.draws.push(ProjectedWorldDraw {
                    receiver: receiver_index,
                    material: &material.material,
                    instances,
                });
                if output.projections.len() == 256 {
                    break;
                }
            }
        }
    }
    Ok(output)
}

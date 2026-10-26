//! Constraint adjacency used by c932d0 when a body enters the physics world.
//! Link allocation and deactivation-island mutation remain world-owned.
use super::body_storage::BodyStorage;

#[derive(Clone, Copy, Debug)]
pub struct ConstraintLink {
    pub bodies: [u32; 2],
    pub flags: u8,
    pub link_index: usize,
}

/// The manager's +70 per-body heads, +08 constraint records and +80 next pairs.
/// Native lists end at 0x7fffffff; adjacency sides follow full body IDs.
pub struct ConstraintLinks<'a> {
    pub heads: &'a [u32],
    pub constraints: &'a [ConstraintLink],
    pub next: &'a [[u32; 2]],
}

impl ConstraintLinks<'_> {
    pub fn activate_neighbors(
        &self,
        enabled: bool,
        bodies: &BodyStorage,
        body: u32,
        mut activate: impl FnMut(u32, u32) -> Result<(), String>,
    ) -> Result<(), String> {
        if !enabled {
            return Ok(());
        }
        let Some(&head) = self.heads.get((body & 0x00ff_ffff) as usize) else {
            return Ok(());
        };
        let mut current = head;
        let mut visited = 0;
        while current != 0x7fff_ffff {
            if visited >= self.constraints.len() {
                return Err("cyclic constraint adjacency".into());
            }
            visited += 1;
            let constraint = self
                .constraints
                .get(current as usize)
                .ok_or("constraint adjacency references a missing record")?;
            let side = usize::from(constraint.bodies[0] != body);
            if constraint.flags & 4 == 0 {
                let other = constraint.bodies[1 - side];
                if let Some(neighbor) = bodies.get(other) {
                    if neighbor.flags & 3 != 0 && neighbor.broadphase_id != u32::MAX {
                        // ASM preserves EDX=body and sets R8D=other for cd8310.
                        activate(body, other)?;
                    }
                }
            }
            current = self
                .next
                .get(constraint.link_index)
                .ok_or("constraint adjacency references a missing next pair")?[side];
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "constraint_links/tests.rs"]
mod tests;

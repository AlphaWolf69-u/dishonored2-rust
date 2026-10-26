//! c93410 collects cross-motion overlap pairs before deactivation updates.
use crate::gameplay::{body_storage::BodyStorage, broadphase::PackedBodyBounds};

#[cfg(test)]
#[path = "contacts_tests.rs"]
mod tests;

/// The query is the world's real packed-AABB broadphase traversal. Preserve
/// duplicates, full IDs and the source's advancing index after swap removal.
pub fn prepare_contacts(
    bodies: &BodyStorage,
    ids: &[u32],
    pairs: &mut Vec<[u32; 2]>,
    mut query: impl FnMut(PackedBodyBounds, &mut Vec<u32>) -> Result<(), String>,
) -> Result<bool, String> {
    pairs.clear();
    let mut retained = Vec::new();
    let mut hits = Vec::new();
    for &id in ids {
        let body = bodies
            .get_by_index(id & 0xffffff)
            .ok_or("deactivation-query body is missing")?;
        hits.clear();
        query(body.bounds, &mut hits)?;
        let mut count = hits.len();
        let mut index = 0;
        while index < count {
            let hit = hits[index];
            let other = bodies
                .get_by_index(hit & 0xffffff)
                .ok_or("deactivation-query hit body is missing")?;
            if body.motion_id == other.motion_id {
                if other.flags & 8 != 0 {
                    count -= 1;
                    if count != index {
                        hits[index] = hits[count];
                    }
                }
            } else {
                pairs.push([id, hit]);
            }
            // The swapped-in hit is deliberately not revisited by c93410.
            index += 1;
        }
        retained.extend_from_slice(&hits[..count]);
    }
    Ok(true)
}

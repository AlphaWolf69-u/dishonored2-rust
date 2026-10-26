//! Acoustic path projection submitted to Wwise by 1400c2b60/1400c3a00.
use super::paths::{Path, Results};
use bevy_math::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub position: Vec3,
    pub front: Vec3,
}

/// 1400c3a00 appends the closest path, then every listener-side portal entry
/// below five times the query's best score. Entries retain their native order
/// and duplicates; this is not a set of unique portal positions.
fn append(output: &mut Vec<Position>, results: &Results, front: Vec3, mixing: bool) -> Result<(), String> {
    if let Some(index) = results.closest {
        let path = results.paths.get(index).ok_or("closest acoustic path missing")?;
        output.push(Position { position: path.position, front });
    }
    let cutoff = results.score * 5.0; // 141abf154
    for entry in &results.portal_paths {
        let index = entry.path.ok_or("position projection references an invalid portal path")?;
        let path = results.paths.get(index).ok_or("position projection references a missing path")?;
        // 1400b94f0/1400b97e0 and the direct solver do not initialize this
        // field. Its retained allocation contents must be established before
        // projecting such an entry; substituting the query score is incorrect.
        let score = match path.score {
            Some(score) => score,
            // Mode 2 mixes rays by per-channel maximum (1410cbbb0,
            // 141104200). An optional duplicate of an already selected
            // position cannot change that maximum or the filter minima.
            // Keep exact submission capture separate: its count is unknown.
            None if mixing && output.iter().any(|selected| selected.position == path.position && selected.front == front) => continue,
            None => return Err("position projection reads an unset direct-path score".into()),
        };
        if score < cutoff {
            output.push(Position { position: path.position, front });
        }
    }
    Ok(())
}

/// 1400c2b60 submits mode 2 with one shared owner orientation. Beyond 75%
/// of the object range, only the global closest path is submitted. Otherwise
/// it concatenates each listener's selection and narrows the count to u16.
pub fn project(
    listeners: &[&Results],
    closest: Option<&Path>,
    range: f32,
    front: Vec3,
) -> Result<Vec<Position>, String> {
    if let Some(path) = closest {
        // COMISS/SETA at 1400c2bab takes this branch only on ordered >.
        if path.distance / range > 0.75 { // 141ac8fcc
            return Ok(vec![Position { position: path.position, front }]);
        }
    }
    let mut output = Vec::new();
    for results in listeners {
        append(&mut output, results, front, false)?;
    }
    output.truncate(usize::from(output.len() as u16));
    Ok(output)
}

/// Render-equivalent mode-2 positions. Unknown membership is permitted only
/// for duplicates that cannot change the mixer result. No score is invented.
/// The exact submission API above remains available for runtime comparison.
pub fn for_mixing(listeners: &[&Results], closest: Option<&Path>, range: f32, front: Vec3) -> Result<Vec<Position>, String> {
    if closest.is_some_and(|path| path.distance / range > 0.75) {
        return project(listeners, closest, range, front);
    }
    let maximum = listeners.iter().try_fold(0usize, |total, result|
        total.checked_add(result.portal_paths.len())?.checked_add(usize::from(result.closest.is_some())))
        .ok_or("acoustic position count overflow")?;
    if maximum > usize::from(u16::MAX) {
        // Optional duplicates could change the narrowed submission count;
        // prove the exact list before attempting any render equivalence.
        return project(listeners, closest, range, front);
    }
    let mut output = Vec::new();
    for results in listeners { append(&mut output, results, front, true)?; }
    Ok(output)
}

//! 325130 reads r_OIT and the OIT weight cvars for every alpha-list draw.
//! Material uniforms carry the resolved values; this owner re-resolves them
//! for every alpha-list material only when the console values change.
use super::ProbeMaterial;
use crate::rendering::particles::transparency::TransparencyParameters;
use bevy::prelude::*;

type Resolved = [(i32, [f32; 4], [f32; 4]); 2];

fn resolve() -> Result<Resolved, String> {
    let console = crate::platform::console::shared().read();
    let one = |additive| {
        TransparencyParameters::resolve(&console.cvars, additive, false)
            .map(|p| (p.mode, p.weights, p.parameters))
    };
    Ok([one(false)?, one(true)?])
}

pub(super) fn refresh(mut materials: ResMut<Assets<ProbeMaterial>>, mut current: Local<Option<Resolved>>) {
    let resolved = match resolve() {
        Ok(resolved) => resolved,
        Err(_) => return,
    };
    // Materials resolve the same values at creation; the first observation
    // is the baseline.
    let Some(previous) = current.replace(resolved) else {
        return;
    };
    if previous == resolved {
        return;
    }
    let ids: Vec<_> = materials
        .iter()
        .filter(|(_, material)| material.extension.surface.oit_mode.y != 0)
        .map(|(id, _)| id)
        .collect();
    for id in ids {
        if let Some(mut material) = materials.get_mut(id) {
            let surface = &mut material.extension.surface;
            let (mode, weights, parameters) = resolved[(surface.oit_mode.y - 1) as usize];
            surface.oit = [Vec4::from_array(weights), Vec4::from_array(parameters)];
            surface.oit_mode.x = mode as u32;
        }
    }
}

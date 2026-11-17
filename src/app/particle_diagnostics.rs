//! F8-only borrowed receipts of existing simulation and draw publications.
use bevy::prelude::*;
use std::fmt::Write;
use crate::rendering::bevy_backend::{camera_scene::{FxPublication, FxSimulation},
    water_scene::WaterSceneFrame, world_effects::{DynamicEffects, WorldEffectFrames}};

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0]-b[0]).powi(2)+(a[1]-b[1]).powi(2)+(a[2]-b[2]).powi(2)
}

pub(super) fn write(world: &World, eye: [f32; 3], report: &mut String) {
    let mut relevant_materials = std::collections::HashSet::new();
    let _ = writeln!(report, "\nParticle ribbon receipt (F8 only; borrowed current owners; nearest 8 per section):");
    let view = world.get_resource::<WaterSceneFrame>().and_then(|frame| frame.0.as_ref());
    let _ = writeln!(report, "  view_clock={:?}", view.map(|frame| (frame.matrices.frame, frame.local_time)));
    if let (Some(effects), Some(frames)) = (world.get_resource::<DynamicEffects>(), world.get_resource::<WorldEffectFrames>()) {
        let mut entries: Vec<_> = effects.0.iter().filter_map(|effect| {
            let parent = frames.resolve(&effect.target, &effect.tag);
            let pose = match effect.initial_world { Some(pose) => Some(pose), None => parent.as_ref().ok().copied().flatten()
                .map(|parent| crate::gameplay::reference_frame::compose(parent, effect.local)) };
            pose.map(|pose| (distance(eye, pose.position), effect, parent, pose))
        }).collect();
        entries.sort_by(|a,b| a.0.total_cmp(&b.0));
        for (_, effect, parent, pose) in entries.into_iter().take(8) {
            let _ = writeln!(report, "  fx owner={} revision={} effect={} target={:?} tag={} attached={} parent={parent:?} local={:?} initial_world={:?} binding_pose={pose:?}",
                effect.owner, effect.revision, effect.effect, effect.target, effect.tag, effect.attached, effect.local, effect.initial_world);
        }
    }
    if let Some(simulation) = world.get_resource::<FxSimulation>() {
        let mut systems: Vec<_> = simulation.diagnostic_particle_systems().filter_map(|(slot, system)| {
            let publication = system.publication.as_ref()?;
            let relevant = system.stages.iter().any(|stage| stage.declaration.stages[stage.index].sprites.iter()
                .any(|sprite| sprite.enabled && matches!(sprite.orientation.mode,
                    crate::gameplay::particles::orientation::OrientationMode::CustomZ |
                    crate::gameplay::particles::orientation::OrientationMode::VelocityZ)))
                || system.materials.values().any(|material| material.scrolling_water);
            relevant.then_some((distance(eye, publication.pose.position), slot, system, publication))
        }).collect();
        systems.sort_by(|a,b| a.0.total_cmp(&b.0));
        for (_, slot, system, publication) in systems.into_iter().take(8) {
            let _ = writeln!(report, "  system slot={slot} name={} declarations={:?} pose={:?} visible={} delta={} colour={:?}",
                system.effect.definition.name, system.effect.definition.declarations, publication.pose,
                publication.visible, publication.simulation_delta, publication.colour);
            for stage in system.stages.iter().take(4) {
                let definition = &stage.declaration.stages[stage.index];
                for (sprite_index, sprite) in definition.sprites.iter().enumerate().filter(|(_, sprite)| sprite.enabled).take(4) {
                    if matches!(sprite.orientation.mode, crate::gameplay::particles::orientation::OrientationMode::CustomZ |
                        crate::gameplay::particles::orientation::OrientationMode::VelocityZ) {
                        if let Some(name) = sprite.material.as_deref() { relevant_materials.insert(name); }
                    }
                    let _ = writeln!(report, "    stage={} worldspace={} owner_pose={:?} sprite={} material={:?} mode={:?} primary_range={:?} primary_worldspace={} initial_pivot_range={:?} life_pivot_range={:?}",
                        stage.index, definition.worldspace, stage.owner_pose, sprite.name, sprite.material, sprite.orientation.mode,
                        sprite.orientation.primary.range, sprite.orientation.primary_worldspace,
                        sprite.spawn.pivot.range, sprite.appearance.pivot_offset_over_life.range);
                    for (particle_slot, particle) in stage.particles.iter().take(2) {
                        let initial = stage.sprites.get(sprite_index).and_then(|states| states.get(particle_slot)).and_then(Option::as_ref);
                        let _ = writeln!(report, "      particle slot={particle_slot} age={} position={:?} velocity={:?} seed={} initial={:?}",
                            particle.state.age, particle.state.position, particle.state.velocity, particle.seed,
                            initial.map(|value| (value.size, value.pivot, value.rotation, value.flip_bits)));
                    }
                }
            }
        }
    }
    let Some((frame, draws, opaque)) = world.get_resource::<FxPublication>().and_then(FxPublication::diagnostic_particle_draws) else {
        let _ = writeln!(report, "  published_draws=absent"); return;
    };
    let time = view.filter(|view| view.matrices.frame == frame).map(|view| view.local_time[0]);
    let mut instances: Vec<(f32, _, _)> = Vec::with_capacity(8);
    for draw in draws.iter().chain(opaque).filter(|draw| draw.mesh.is_none() && draw.strip.is_empty()
            && (draw.material.scrolling_water || relevant_materials.contains(draw.material.name.as_str()))) {
        for instance in &draw.instances {
            let distance = distance(eye, instance.position);
            let index = instances.partition_point(|candidate| candidate.0.total_cmp(&distance).is_lt());
            if index < 8 {
                if instances.len() == 8 { instances.pop(); }
                instances.insert(index, (distance, draw, instance));
            }
        }
    }
    for (_, draw, instance) in instances {
        let word = instance.parameters.flipbook_seed_uv;
        let uv = |u: f32, v: f32| [if word & (1<<30)!=0 {1.-u} else {u}, if word & (1<<31)!=0 {1.-v} else {v}];
        let corners = [(-1.,1.),(1.,1.),(-1.,-1.),(1.,-1.)].map(|(y,z)| std::array::from_fn::<_,3,_>(|i|
            instance.position[i]+instance.axes[1][i]*y+instance.axes[2][i]*z));
        let _ = writeln!(report, "  draw frame={frame} material={} scrolling_water={} grid={} centre={:?} axes={:?} corners={corners:?} base_uv={:?} packed_uv={word:08x} same_frame_time={time:?} opacity_pan={:?} disto1_pan={:?} disto2_pan={:?} clip={:?}",
            draw.material.name, draw.material.scrolling_water, draw.grid_level, instance.position, instance.axes,
            [uv(0.,0.),uv(1.,0.),uv(0.,1.),uv(1.,1.)], draw.material.uniform("opacity_PanningSpeed"),
            draw.material.uniform("disto1_PanningSpeed"), draw.material.uniform("disto2_PanningSpeed"), draw.clip_plane);
    }
}

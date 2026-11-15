//! Drive every loadable installed beam emitter along a moving system and
//! build its strip, reporting failures, vertex counts and non-finite output:
//! `cargo run --features game --example beam_smoke`.
use dishonored2_rust::gameplay::{
    particles::{beam::BeamTarget, beam_noise::SimplexTable, beam_system::BeamEmitter},
    reference_frame::ReferenceFrame,
};

fn main() {
    let base = dishonored2_rust::assets::installation::game_base().unwrap();
    let catalog = dishonored2_rust::archive::Catalog::open(&base).unwrap();
    let noise = SimplexTable::load(&base).unwrap();
    let names: Vec<String> = catalog.paths_with_suffix(".decl").into_iter()
        .filter_map(|p| p.strip_prefix("generated/decls/fx/").and_then(|p| p.strip_suffix(".decl")).map(str::to_owned))
        .collect();
    let mut programs = dishonored2_rust::shader_cache::ProgramLibrary::open(
        &base, dishonored2_rust::shader_cache::ShaderCacheBackend::Pc, &catalog).unwrap();
    let mut resources = dishonored2_rust::rendering::particles::definition::ParticleResources::default();
    let sources = dishonored2_rust::rendering::particles::world_sources::WorldFxSources::load_named_shared(
        names, &catalog, &mut programs, &mut resources);
    let declarations: Vec<_> = sources.effects.values().filter_map(|e| e.as_ref().ok())
        .flat_map(|e| &e.groups).flat_map(|g| &g.particles).filter_map(|p| p.as_ref().ok())
        .flat_map(|p| p.declarations.iter().flatten().cloned()).collect();
    let mut seen = std::collections::HashSet::new();
    let (mut beams, mut failures, mut drawn, mut vertices, mut bad) = (0, 0, 0, 0usize, 0);
    for declaration in declarations {
        if !seen.insert(std::sync::Arc::as_ptr(&declaration)) {
            continue;
        }
        for asset in declaration.beams.iter().filter(|b| b.enabled && b.material.is_some()) {
            beams += 1;
            let result = (|| -> Result<(usize, bool), String> {
                let mut seed = 0x8765_4321;
                let frame = |t: f32| ReferenceFrame {
                    position: [t * 2., (t * 3.).cos(), 1.],
                    basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                };
                let mut emitter = BeamEmitter::new(asset, &mut seed, frame(0.))?;
                let mut strip = Vec::new();
                let mut finite = true;
                let mut peak = 0usize;
                for step in 1..=180 {
                    let t = step as f32 / 60.;
                    emitter.advance(asset, &declaration, 1. / 60., frame(t), &BeamTarget::default(), [0.5, 0., 0.],
                        None, 1., true)?;
                    strip.clear();
                    let bounds = emitter.build(asset, [0., -5., 1.], [1.; 4], frame(t), Some(&noise), &mut strip)?;
                    peak = peak.max(strip.len());
                    finite &= strip.iter().all(|v| v.position.iter().all(|c| c.is_finite()));
                    finite &= !(bounds[0][0] <= bounds[1][0]) || bounds.iter().flatten().all(|c| c.is_finite());
                }
                if peak == 0 { println!("{}: never drawn (delay {:?}, burst {}, cycles {})", asset.name, asset.timings.emitter.delay, asset.timings.burst, asset.timings.emitter.cycles); }
                Ok((strip.len(), finite))
            })();
            match result {
                Ok((count, finite)) => {
                    vertices += count;
                    drawn += usize::from(count != 0);
                    if !finite {
                        bad += 1;
                        println!("{}: non-finite strip", asset.name);
                    }
                }
                Err(error) => {
                    failures += 1;
                    println!("{}: {error}", asset.name);
                }
            }
        }
    }
    println!("beams={beams} failures={failures} drawn={drawn} non_finite={bad} final_vertices={vertices}");
    let (mut ribbons, mut failures, mut drawn, mut bad) = (0, 0, 0, 0);
    let mut seen = std::collections::HashSet::new();
    for declaration in sources.effects.values().filter_map(|e| e.as_ref().ok())
        .flat_map(|e| &e.groups).flat_map(|g| &g.particles).filter_map(|p| p.as_ref().ok())
        .flat_map(|p| p.declarations.iter().flatten().cloned())
    {
        if !seen.insert(std::sync::Arc::as_ptr(&declaration)) {
            continue;
        }
        for asset in declaration.ribbons.iter().filter(|r| r.enabled && r.material.is_some()) {
            ribbons += 1;
            let result = (|| -> Result<(usize, bool), String> {
                let mut seed = 0x8765_4321;
                let frame = |t: f32| ReferenceFrame {
                    position: [t * 2., (t * 3.).cos(), 1.],
                    basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                };
                let mut ribbon = dishonored2_rust::gameplay::particles::ribbon::Ribbon::new(asset, &mut seed)?;
                let (mut strip, mut peak, mut finite) = (Vec::new(), 0usize, true);
                for step in 1..=180 {
                    let t = step as f32 / 60.;
                    ribbon.advance(asset, &declaration, 1. / 60., frame(t), [0.5, 0., 0.], None, 1., true)?;
                    strip.clear();
                    ribbon.build(asset, [0., -5., 1.], [1.; 4], frame(t), Some(&noise), &mut strip)?;
                    peak = peak.max(strip.len());
                    finite &= strip.iter().all(|v| v.position.iter().all(|c| c.is_finite()));
                }
                Ok((peak, finite))
            })();
            match result {
                Ok((peak, finite)) => {
                    drawn += usize::from(peak != 0);
                    if peak == 0 { println!("{}: ribbon never drawn", asset.name); }
                    if !finite { bad += 1; println!("{}: non-finite ribbon", asset.name); }
                }
                Err(error) => { failures += 1; println!("{}: {error}", asset.name); }
            }
        }
    }
    println!("ribbons={ribbons} failures={failures} drawn={drawn} non_finite={bad}");
}

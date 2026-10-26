use dishonored2_rust::assets::wwise::Package;
use std::{collections::BTreeMap, env, error::Error, fs::File, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let first = arguments.next().ok_or("Usage: wwise-audit [--summary | --object ID | --resolve-events] <Wwise package.pck>")?;
    if first.to_string_lossy() == "--container-playback" {
        use dishonored2_rust::assets::{installation, wwise::{bank_container_playback, bank_play_settings, bank_state_actions, TransitionMode}};
        let path = arguments.next().map(PathBuf::from).unwrap_or(installation::game_base()?.join("pck"));
        let mut paths = if path.is_dir() {
            std::fs::read_dir(path)?.map(|entry| entry.map(|entry| entry.path())).collect::<Result<Vec<_>, _>>()?
                .into_iter().filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pck"))).collect()
        } else { vec![path] };
        paths.sort();
        let mut totals = BTreeMap::<String, usize>::new();
        for path in paths {
            let package = Package::open(&path)?;
            let mut file = File::open(&path)?;
            for entry in package.banks.iter().filter(|entry| entry.language_id == 0) {
                let bytes = package.read_entry(&mut file, *entry)?;
                for (_, state) in bank_state_actions(&bytes)? {
                    *totals.entry("Set State actions".into()).or_default() += 1;
                    if state.delay.frames != 0 || state.delay.modifiers != [0; 2] {
                        *totals.entry("Set State actions with a delay".into()).or_default() += 1;
                    }
                }
                for (_, settings) in bank_play_settings(&bytes)? {
                    *totals.entry("Play actions".into()).or_default() += 1;
                    for (name, present) in [
                        ("Play actions with a fade", settings.transition.milliseconds != 0 || settings.transition.modifiers != [0; 2]),
                        ("Play actions with a delay", settings.delay.frames != 0 || settings.delay.modifiers != [0; 2]),
                        ("Play actions with probability", settings.probability.is_some()),
                    ] { if present { *totals.entry(name.into()).or_default() += 1; } }
                }
                for (id, settings) in bank_container_playback(&bytes)? {
                    *totals.entry(format!("continuous={} transition={:?}", settings.continuous, settings.transition)).or_default() += 1;
                    if settings.continuous && matches!(settings.transition, TransitionMode::CrossfadeAmplitude | TransitionMode::CrossfadePower) {
                        println!("{} bank={:#010x} container={id:#010x}: {settings:?}", path.display(), entry.id);
                    }
                }
            }
        }
        for (kind, count) in totals { println!("{kind}: {count}"); }
        return Ok(());
    }
    if first.to_string_lossy() == "--level-acoustics" {
        use dishonored2_rust::assets::{archive::Catalog, installation, level::{LevelResources, DEFAULT_LEVEL}, sound_propagation::Propagation};
        let requested = arguments.next().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| DEFAULT_LEVEL.to_owned());
        let catalog = Catalog::open(&installation::game_base()?)?;
        let level = LevelResources::resolve(&catalog, &requested)?;
        let path = format!("{}.soundpropa", level.entities.trim_end_matches(".entities"));
        let bytes = catalog.read(&path)?.ok_or_else(|| format!("missing installed resource {path}"))?;
        let propagation = Propagation::parse(&bytes)?;
        println!("{path}: {} zones, {} portals, route dimension {}", propagation.zones.len(), propagation.portals.len(), propagation.routes.zone_count);
        let mut route_count = 0usize;
        let mut connected_pairs = 0usize;
        for (index, zone) in propagation.zones.iter().enumerate() {
            println!("zone={index} id={} portal_capacity={} auxiliary_bus={:?} properties={:?} shapes={}",
                zone.id, zone.portal_capacity, zone.auxiliary_bus, zone.properties, zone.geometry.iter().count());
            for other in &propagation.zones[index + 1..] {
                let routes = propagation.routes.between(zone.id, other.id)?;
                connected_pairs += usize::from(!routes.is_empty());
                route_count += routes.len();
                if routes.iter().flat_map(|route| &route.portals).any(|&portal| usize::from(portal) >= propagation.portals.len()) {
                    return Err(format!("route {} to {} references missing portal", zone.id, other.id).into());
                }
            }
        }
        println!("{connected_pairs} connected zone pairs, {route_count} authored paths");
        let flags = arguments.collect::<Vec<_>>();
        let inspect_audio = flags.iter().any(|argument| argument == "--audio");
        if inspect_audio || flags.iter().any(|argument| argument == "--player") {
            let bytes = level.read_entities(&catalog)?.ok_or("level has no entities")?;
            let document = dishonored2_rust::entities::parse(&String::from_utf8_lossy(&bytes))?;
            let player = dishonored2_rust::entities::player_start(&document).ok_or("level has no player start")?;
            let mut world = dishonored2_rust::gameplay::sound::acoustic_world::AcousticWorld::new(propagation, &installation::game_base()?)?;
            let listener_zone = world.zone_at(player.position)?;
            println!("authored player start {:?}: acoustic zone {:?}", player.position, listener_zone);
            if inspect_audio {
                use dishonored2_rust::gameplay::sound::{admission::{Admission, Query}, paths, propagation};
                use dishonored2_rust::gameplay::sound::auxiliary;
                let auxiliary_buses = auxiliary::Buses::load(&world.graph, |path| {
                    catalog.read(path).map(|bytes| bytes.map(|bytes| dishonored2_rust::entities::declaration_text(&bytes).into_owned())).map_err(|error| error.to_string())
                })?;
                let detail = dishonored2_rust::platform::console::shared().read().cvars.int("s_environmentalDetailLevel")
                    .ok_or("environmental audio detail cvar is absent")?;
                let bindings = dishonored2_rust::entities::audio::audio_bindings(&document, |path| {
                    catalog.read(path).map(|bytes| bytes.map(|bytes| dishonored2_rust::entities::declaration_text(&bytes).into_owned())).map_err(|error| error.to_string())
                })?;
                let mut deferred = 0usize;
                let mut admitted = 0usize;
                let mut active = 0usize;
                let mut projected = 0usize;
                let mut mixable = 0usize;
                for binding in bindings {
                    let startup = binding.startup();
                    if !startup.deferred { continue; }
                    deferred += 1;
                    let source_zone = world.zone_at(binding.frame.position)?;
                    let query = Query { source: binding.frame.position.into(), listener: player.position.into(),
                        range: startup.max_distance, vertical_scale: 1.0, propagation: binding.propagation,
                        source_zone, listener_zone };
                    let admission = propagation::admission(&world.graph, &query, 1.0)?;
                    if admission == Admission::Inaudible { continue; }
                    admitted += 1;
                    let result = paths::evaluate(&world.graph, &paths::Query {
                        source: query.source, listener: query.listener, source_zone, listener_zone,
                        range: query.range, propagation: query.propagation, reflection_normal: None,
                        source_occlusion_scale: binding.settings.source_occlusion_scale(),
                    })?;
                    active += usize::from(result.closest.is_some());
                    println!("emitter={} slot={} source_zone={source_zone:?} admission={admission:?} paths={} closest={:?} score={}",
                        binding.entity_id, binding.slot, result.paths.len(), result.closest, result.score);
                    let closest = result.closest.and_then(|index| result.paths.get(index));
                    let sends = auxiliary::project(&world.graph, closest, binding.settings.auxiliary_sends,
                        detail, |name| auxiliary_buses.resolve(name))?;
                    println!("  environmental sends: {sends:?}");
                    match dishonored2_rust::gameplay::sound::positions::project(
                        &[&result], closest, query.range, binding.frame.basis[0].into(),
                    ) {
                        Ok(positions) => {
                            projected += 1;
                            println!("  playback positions: {positions:?}");
                        }
                        Err(error) => println!("  playback position projection unresolved: {error}"),
                    }
                    match dishonored2_rust::gameplay::sound::positions::for_mixing(
                        &[&result], closest, query.range, binding.frame.basis[0].into(),
                    ) {
                        Ok(positions) => {
                            mixable += 1;
                            println!("  mode-2 mixing positions: {positions:?}");
                        }
                        Err(error) => println!("  mode-2 mixing unresolved: {error}"),
                    }
                }
                println!("{deferred} declared deferred loops, {admitted} admitted origins, {active} audible active results at player start");
                println!("{projected} emitter position lists resolved from authored acoustic paths");
                println!("{mixable} emitter position lists resolved for mode-2 mixing");
            }
            world.release()?;
        }
        return Ok(());
    }
    if first.to_string_lossy() == "--level-audio" {
        use dishonored2_rust::{archive::Catalog, assets::{installation, level::{LevelResources, DEFAULT_LEVEL}}, entities};
        let requested = arguments.next().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| DEFAULT_LEVEL.to_owned());
        let catalog = Catalog::open(&installation::game_base()?)?;
        let level = LevelResources::resolve(&catalog, &requested)?;
        let bytes = level.read_entities(&catalog)?.ok_or("level has no entity resource")?;
        let document = entities::parse(&String::from_utf8_lossy(&bytes))?;
        let bindings = entities::audio::audio_bindings(&document, |path| {
            catalog.read(path).map(|value| value.map(|bytes| entities::declaration_text(&bytes).into_owned())).map_err(|error| error.to_string())
        })?;
        println!("{}: {} audio components", level.entities, bindings.len());
        for binding in bindings {
            println!("entity={} slot={} spawn={} phases={:#x} propagation={} settings={:?} startup={:?}",
                binding.entity_id, binding.slot, binding.scheduling.starts_on_spawn(), binding.scheduling.start_phases,
                binding.propagation, binding.settings, binding.startup());
            for event in binding.events.iter().flatten() {
                println!("  event={:#010x} loop={} spatial={} range={} duration={:?} game_properties={:?}",
                    event.id, event.persistent_loop(), event.spatialized, event.max_distance, event.duration, event.game_properties);
            }
        }
        return Ok(());
    }
    if first.to_string_lossy() == "--attenuations" {
        let path = arguments.next().map(PathBuf::from).ok_or("missing package path")?;
        let wanted = arguments.next().map(|id| u32::from_str_radix(id.to_string_lossy().trim_start_matches("0x"), 16)).transpose()?;
        let package = Package::open(&path)?;
        let mut file = File::open(&path)?;
        for entry in &package.banks {
            for attenuation in dishonored2_rust::assets::wwise::bank_attenuations(&package.read_entry(&mut file, *entry)?)? {
                if wanted.is_some_and(|id| id != attenuation.id) { continue; }
                println!("bank={:#x} {attenuation:#?}", entry.id);
            }
        }
        return Ok(());
    }
    if first.to_string_lossy() == "--mix" {
        let path = match arguments.next() {
            Some(path) => PathBuf::from(path),
            None => dishonored2_rust::assets::installation::game_base()?.join("pck/main-init.pck"),
        };
        let package = Package::open(&path)?;
        let mut file = File::open(&path)?;
        for entry in &package.banks {
            let bytes = package.read_entry(&mut file, *entry)?;
            let Some(mut mix) = dishonored2_rust::assets::wwise::bank_mix(&bytes)? else { continue };
            println!("volume threshold={} dB, {} parameter defaults, {} buses", mix.volume_threshold_db, mix.parameters.len(), mix.buses.len());
            println!("{} global state groups", mix.state_groups.len());
            println!("primary output bus: {:x?}", mix.primary_bus);
            let mut reflection_indices = std::collections::BTreeSet::new();
            for effect in &mix.effects {
                if effect.plugin == dishonored2_rust::gameplay::sound::room_reverb::PLUGIN {
                    let parameters = dishonored2_rust::gameplay::sound::room_reverb::Parameters::read(&effect.parameters)?;
                    if parameters.early_reflections { reflection_indices.insert(parameters.early_pattern); }
                }
            }
            let reflection_patterns = if reflection_indices.is_empty() { None } else {
                Some(dishonored2_rust::assets::sound_effects::ReflectionPatterns::load(
                    &dishonored2_rust::assets::installation::game_base()?, reflection_indices)?)
            };
            let matrix_delays = if mix.effects.iter().any(|effect| effect.plugin == dishonored2_rust::gameplay::sound::matrix_reverb::PLUGIN) {
                Some(dishonored2_rust::assets::sound_effects::matrix_delays(&dishonored2_rust::assets::installation::game_base()?)?)
            } else { None };
            mix.effects.sort_by_key(|effect| (effect.shared, effect.id));
            for effect in &mix.effects {
                println!("effect={:#010x} shared={} plugin={:#010x} parameter_bytes={} media={:?} curves={} values={:?}",
                    effect.id, effect.shared, effect.plugin, effect.parameters.len(), effect.media, effect.curves.len(), effect.values);
                if effect.plugin == dishonored2_rust::gameplay::sound::matrix_reverb::PLUGIN {
                    let parameters = dishonored2_rust::gameplay::sound::matrix_reverb::Parameters::read(&effect.parameters)?;
                    println!("  matrix reverb: {parameters:?}");
                    let lengths = parameters.delay_lengths(48000, matrix_delays.as_ref().ok_or("matrix reverb delay table was not loaded")?)?;
                    println!("  matrix delay lengths at 48 kHz: {lengths:?}, damping={:?}", parameters.damping(&lengths, 48000)?);
                }
                if effect.plugin == dishonored2_rust::gameplay::sound::room_reverb::PLUGIN {
                    let parameters = dishonored2_rust::gameplay::sound::room_reverb::Parameters::read(&effect.parameters)?;
                    println!("  room reverb: {parameters:?}");
                    if parameters.early_reflections {
                        let pattern = reflection_patterns.as_ref().ok_or("room reflection patterns were not loaded")?.get(parameters.early_pattern)?;
                        println!("  early pattern={} left_taps={} right_taps={} first_ms={:?}",
                            parameters.early_pattern, pattern.left.len(), pattern.right.len(),
                            [pattern.left[0].milliseconds, pattern.right[0].milliseconds]);
                    }
                }
            }
            for (group, curves) in ["obstruction", "occlusion"].into_iter().zip(&mix.environment.curves) {
                for (property, curve) in ["volume", "LPF", "HPF"].into_iter().zip(curves) {
                    println!("  {group} {property}: enabled={} scaling={} points={:?}", curve.enabled, curve.scaling, curve.points);
                }
            }
            let mut parameters = mix.parameters.iter().collect::<Vec<_>>();
            parameters.sort_by_key(|(id, _)| **id);
            for (id, value) in parameters { println!("parameter={id:#010x} default={value}"); }
            mix.buses.sort_by_key(|bus| bus.id);
            for bus in &mix.buses {
                println!("bus={:#010x} parent={:#010x} volume={} bus_volume={}", bus.id, bus.parent, bus.float(0), bus.float(5));
                println!("  settings: {:?}", bus.bus_settings);
                if !bus.effects.slots.is_empty() { println!("  effects: {:?}", bus.effects); }
                for curve in &bus.curves {
                    println!("  RTPC={:#010x} default={:?} property={} scaling={} accumulation={} points={:?}",
                        curve.parameter, mix.parameters.get(&curve.parameter), curve.property, curve.scaling, curve.accumulation, curve.points);
                }
            }
            let mixer = dishonored2_rust::gameplay::sound::mix::Mixer::from(mix);
            let tables = dishonored2_rust::gameplay::sound::bus_effects::EffectTables::load(
                &mixer, &dishonored2_rust::assets::installation::game_base()?)?;
            for &id in &mixer.bus_order {
                if !mixer.bus_mixes(id)? { continue; }
                let bus = mixer.buses.get(&id).ok_or("missing mixing bus")?;
                let effects = dishonored2_rust::gameplay::sound::bus_effects::BusEffects::new(bus, &mixer, &tables, 48000, 3,
                    bus.bus_settings.as_ref().ok_or("missing bus settings")?.auxiliary)?;
                println!("prepared stereo mixing bus {id:#010x}: {} effect slots, output_db={}", effects.slot_count(), mixer.bus_output_db(id)?);
            }
        }
        return Ok(());
    }
    if matches!(first.to_string_lossy().as_ref(), "--asset" | "--find-assets") {
        let query = arguments.next().ok_or("missing resource path or substring")?.to_string_lossy().into_owned();
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = dishonored2_rust::archive::Catalog::open(&base)?;
        if first.to_string_lossy() == "--find-assets" {
            for path in catalog.paths_containing(&query) { println!("{path}"); }
        } else {
            let bytes = catalog.read(&query)?.ok_or_else(|| format!("missing installed resource: {query}"))?;
            println!("{}", String::from_utf8_lossy(&bytes));
        }
        return Ok(());
    }
    if first.to_string_lossy() == "--player-events" {
        let character = arguments.next().ok_or("missing player character name")?.to_string_lossy().into_owned();
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = dishonored2_rust::archive::Catalog::open(&base)?;
        let player = dishonored2_rust::entities::player::player_declarations(&character, |path| {
            catalog.read(path).map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())).map_err(|error| error.to_string())
        })?;
        for (state, fields) in player.states {
            for (key, value) in fields {
                if !key.contains("SoundEvent") { continue; }
                if let dishonored2_rust::entities::Value::Atom(path) = value {
                    let path = path.trim_matches('"');
                    if path.is_empty() { continue; }
                    let name = path.rsplit('/').next().unwrap_or(&path);
                    println!("{state} {key} {path} id={:#010x}", dishonored2_rust::assets::wwise::event_id(name));
                }
            }
        }
        return Ok(());
    }
    if first.to_string_lossy() == "--midnight-events" {
        let character = arguments.next().ok_or("missing player character name")?.to_string_lossy().into_owned();
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = dishonored2_rust::archive::Catalog::open(&base)?;
        let player = dishonored2_rust::entities::player::player_declarations(&character, |path| {
            catalog.read(path).map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())).map_err(|error| error.to_string())
        })?;
        use dishonored2_rust::gameplay::{player::{decls, states, midnight::prepare_sounds}, sound::Emitter};
        use dishonored2_rust::entities::scene_motion::{atom, child};
        let read = |path: &str| catalog.read(path)
            .map(|bytes| bytes.map(|bytes| dishonored2_rust::entities::declaration_text(&bytes).into_owned()))
            .map_err(|error| error.to_string());
        let mut casting = vec![
            ("PlayerBody", player.body_model.clone(), Emitter::PlayerBody),
            ("PlayerCamera", player.body_model.clone(), Emitter::PlayerBody),
        ];
        for ((entity, _, _), (name, emitter)) in decls::arm_parts(&player.body)?.into_iter().zip([
            ("PlayerLeftArm", Emitter::PlayerLeftArm), ("PlayerRightArm", Emitter::PlayerRightArm),
        ]) {
            let entity = entity.ok_or("player arm has no entity")?;
            let fields = dishonored2_rust::entities::player::resolved_decl("entitydef", &entity, read)?;
            let model = atom(child(&fields, "renderModelInfo"), "model").ok_or("player arm has no model")?;
            casting.push((name, model.to_owned(), emitter));
        }
        let casting = casting.iter().map(|(name, model, emitter)| (*name, model.as_str(), *emitter)).collect::<Vec<_>>();
        let paths = states::scene_paths(&decls::states(&player)?);
        let paths = paths.into_iter().map(|path| format!("generated/decls/midnightscene/{path}.decl"))
            .collect::<std::collections::BTreeSet<_>>();
        println!("{} referenced player Midnight declarations", paths.len());
        let mut track_types = BTreeMap::<String, usize>::new();
        for path in paths {
            let bytes = catalog.read(&path)?.ok_or_else(|| format!("missing installed Midnight resource: {path}"))?;
            let text = String::from_utf8_lossy(&bytes);
            for (offset, _) in text.match_indices("m_trackType") {
                let tail = &text[offset..(offset + 128).min(text.len())];
                if let Some((_, value)) = tail.split_once('=') {
                    if let Some(start) = value.find('"') {
                        if let Some(end) = value[start + 1..].find('"') {
                            *track_types.entry(value[start + 1..start + 1 + end].to_owned()).or_default() += 1;
                        }
                    }
                }
            }
            let scene = dishonored2_rust::entities::midnight::parse(&text).map_err(|error| format!("{path}: {error}"))?;
            for (index, sounds) in prepare_sounds(&scene, &casting, read)?.iter().enumerate() {
                for sound in sounds {
                    println!("{path} scene={} emitter={:?} time={} stop_on_interruption={} event={} id={:#010x}",
                        scene.scenes[index].uid, sound.emitter, sound.key.time_ms, sound.key.stop_on_interruption, sound.key.event, sound.event);
                }
            }
            for (group, name) in &scene.groups {
                for event in scene.scenes.iter().flat_map(|scene| scene.groups.iter())
                    .filter(|entry| entry.group == *group).flat_map(|entry| &entry.events)
                {
                    println!("{path} group={name} time={} message={:?}", event.time_ms, event.event);
                }
            }
        }
        for (track, count) in track_types { println!("track {track}: {count}"); }
        return Ok(());
    }
    if matches!(first.to_string_lossy().as_ref(), "--find-audio-decls" | "--find-decl-field") {
        let needle = if first.to_string_lossy() == "--find-decl-field" {
            arguments.next().ok_or("missing declaration field")?.to_string_lossy().to_ascii_lowercase()
        } else { "cpntaudio".to_owned() };
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = dishonored2_rust::archive::Catalog::open(&base)?;
        let mut paths = std::collections::BTreeSet::new();
        for suffix in [".def", ".decl", ".entities", ".world"] {
            paths.extend(catalog.paths_with_suffix(suffix));
        }
        let mut matches = 0usize;
        for path in paths {
            let Some(bytes) = catalog.read(&path)? else { continue };
            let text = String::from_utf8_lossy(&bytes);
            if !text.to_ascii_lowercase().contains(&needle) { continue; }
            matches += 1;
            println!("{path}");
            for line in text.lines().filter(|line| line.to_ascii_lowercase().contains(&needle)) {
                println!("  {}", line.trim());
            }
        }
        println!("{matches} installed declarations contain {needle}");
        return Ok(());
    }
    if first.to_string_lossy() == "--find-media" {
        let id = arguments.next().ok_or("missing media ID")?.to_string_lossy().trim_start_matches("0x").to_owned();
        let id = u32::from_str_radix(&id, 16)? as u64;
        let directory = PathBuf::from(arguments.next().ok_or("missing PCK directory")?);
        for item in std::fs::read_dir(directory)? {
            let path = item?.path();
            if path.extension().is_none_or(|extension| extension != "pck") { continue; }
            let Ok(package) = Package::open(&path) else { continue };
            for entry in package.media.iter().chain(&package.external_media).filter(|entry| entry.id == id) {
                println!("{} id={:#x} lang={} size={}", path.display(), entry.id, entry.language_id, entry.size);
            }
        }
        return Ok(());
    }
    if matches!(first.to_string_lossy().as_ref(), "--resolve-event" | "--playback-event" | "--playback-node") {
        let playback = first.to_string_lossy() == "--playback-event";
        let actor = first.to_string_lossy() == "--playback-node";
        let id = arguments.next().ok_or("missing event or actor ID")?.to_string_lossy().trim_start_matches("0x").to_owned();
        let id = u32::from_str_radix(&id, 16)?;
        let directory = match arguments.next() {
            Some(path) => PathBuf::from(path),
            None => dishonored2_rust::assets::installation::game_base()?.join("pck"),
        };
        let initialization = directory.join("main-init.pck");
        let package = Package::open(&initialization)?;
        let mut file = File::open(initialization)?;
        let mut mixer = None;
        for entry in &package.banks {
            if let Some(mix) = dishonored2_rust::assets::wwise::bank_mix(&package.read_entry(&mut file, *entry)?)? {
                mixer = Some(dishonored2_rust::gameplay::sound::mix::Mixer::from(mix));
            }
        }
        let mixer = mixer.ok_or("installed initialization bank has no mix")?;
        let mut paths = std::fs::read_dir(directory)?.filter_map(|entry| entry.ok()).map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pck")))
            .collect::<Vec<_>>();
        paths.sort();
        let mut library = dishonored2_rust::assets::wwise::PlaybackBanks::default();
        let mut owners = Vec::new();
        let base = dishonored2_rust::assets::installation::game_base()?;
        let audio_language = dishonored2_rust::assets::lang::read_audio_language(&base)?;
        let package_language = dishonored2_rust::assets::wwise::language_name(audio_language).ok_or("unsupported audio package language")?;
        println!("Audio language: {audio_language} ({package_language})");
        for path in paths {
            let package = Package::open(&path)?;
            let mut file = File::open(&path)?;
            let language = package.language_id(package_language);
            for entry in package.banks.iter().copied().filter(|entry| entry.language_id == 0 || Some(entry.language_id) == language) {
                library.insert(package.read_entry(&mut file, entry)?.into())?;
                owners.push((path.clone(), package.clone(), entry));
            }
        }
        println!("{} loaded banks, {} global actors, {} events", owners.len(), library.actor_count(), library.event_count());
        println!("Loaded reference ownership: {:?}", library.references()?);
        if !actor && library.event_ids().binary_search(&id).is_err() {
            println!("event {id:#010x} is not present in the selected installed banks");
            return Ok(());
        }
        let mut files = std::collections::HashMap::<usize, File>::new();
        let media = |owner: usize, media_id| {
            let (path, package, entry) = &owners[owner];
            let Some(media) = package.media_file(u64::from(media_id), entry.language_id) else { return Ok(None); };
            let file = match files.entry(owner) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(File::open(path)?),
            };
            println!("media {media_id:#010x}: {} bank={:#010x}", path.display(), entry.id);
            package.read_entry(file, media).map(Some)
        };
        if actor {
            if let Some(node) = library.actor_playback(id, media)? {
                print_playback(&node, 0, &mixer, None)?;
            } else { println!("actor {id:#010x} is not available for playback"); }
            return Ok(());
        }
        let actions = library.event_playback(id, media)?;
        let Some(actions) = actions else {
            println!("event {id:#010x} has no supported Wwise actions");
            return Ok(());
        };
        if playback {
            for action in actions {
                use dishonored2_rust::assets::wwise::PlaybackCommand;
                match action {
                    PlaybackCommand::Failed { action, target, reason } => println!("  Failed action={action:#010x} target={target:#010x}: {reason}"),
                    PlaybackCommand::SetState(state) => println!("  Set State group={:#010x}, value={:#010x}, {:?}", state.group, state.value, state.delay),
                    PlaybackCommand::Stop(stop) => println!("  Stop target={:#010x}, all={}, game_object={}, {:?}", stop.target, stop.all, stop.game_object, stop.transition),
                    PlaybackCommand::Play { target, settings, node } => {
                        println!("  Play target={target:#010x}, {settings:?}");
                        print_playback(&node, 2, &mixer, None)?;
                    },
                }
            }
        } else {
            for action in dishonored2_rust::assets::wwise::flatten_playback_actions(actions)? {
                use dishonored2_rust::assets::wwise::EventCommand;
                match action {
                    EventCommand::Stop(stop) => println!("  Stop target={:#010x}, all={}, game_object={}, {:?}", stop.target, stop.all, stop.game_object, stop.transition),
                    EventCommand::SetState(state) => println!("  Set State group={:#010x}, value={:#010x}, {:?}", state.group, state.value, state.delay),
                    EventCommand::Play { target, settings, groups } => {
                        println!("  Play target={target:#010x}, {settings:?}");
                        for (group_index, group) in groups.iter().enumerate() {
                            for (variant_index, variant) in group.variants.iter().enumerate() {
                                println!("    group {group_index} {:?} variant {variant_index} weight {} bus={:#010x} default_mix_db={} WEM {} bytes",
                                    group.mode, variant.weight, variant.properties.bus, mixer.volume_db(&variant.properties)?, variant.media.len());
                                let audio = dishonored2_rust::assets::wwise::decode_wem_pcm(&variant.media)?;
                                println!("      decoded {} samples, {} channels at {} Hz", audio.samples.len(), audio.channels, audio.sample_rate);
                            }
                        }
                    },
                }
            }
        }
        return Ok(());
    }
    let (summary_only, object_filter, resolve_events, path) = match first.to_string_lossy().as_ref() {
        "--summary" => (true, None, false, arguments.next().map(PathBuf::from).ok_or("missing package path")?),
        "--object" => {
            let id = arguments.next().ok_or("missing object ID")?.to_string_lossy().trim_start_matches("0x").to_owned();
            let id = u32::from_str_radix(&id, 16)?;
            (false, Some(id), false, arguments.next().map(PathBuf::from).ok_or("missing package path")?)
        }
        "--resolve-events" => (false, None, true, arguments.next().map(PathBuf::from).ok_or("missing package path")?),
        _ => (false, None, false, PathBuf::from(first)),
    };
    let package = Package::open(&path)?;
    println!("{}", path.display());
    println!("{} bytes, data starts at {:#x}", package.file_size(), package.data_offset);
    println!("Languages:");
    for language in &package.languages {
        println!("  {} = {}", language.id, language.name);
    }
    println!("{} banks, {} media items, {} external media items", package.banks.len(), package.media.len(), package.external_media.len());
    for entry in &package.banks {
        println!("  bank id={:#x} lang={} offset={:#x} size={}", entry.id, entry.language_id, entry.offset, entry.size);
    }
    if summary_only {
        return Ok(());
    }
    for entry in package.media.iter().chain(&package.external_media) {
        println!("  id={:#x} lang={} offset={:#x} size={}", entry.id, entry.language_id, entry.offset, entry.size);
    }
    let mut file = File::open(path)?;
    let mut package_media = std::collections::HashMap::new();
    if resolve_events {
        let language_id = package.banks.first().map_or(0, |bank| bank.language_id);
        for item in package.media.iter().chain(&package.external_media)
            .filter(|item| item.language_id == language_id)
        {
            package_media.insert(item.id as u32, package.read_entry(&mut file, *item)?);
        }
    }
    for entry in &package.banks {
        let bank = package.read_entry(&mut file, *entry)?;
        if resolve_events {
            for id in event_ids(&bank)? {
                match dishonored2_rust::assets::wwise::event_media(&bank, id, &package_media) {
                    Err(error) => println!("  event {id:#010x} -> error: {error}"),
                    Ok(Some(groups)) => {
                        for (group_index, group) in groups.iter().enumerate() {
                            for (variant_index, variant) in group.variants.iter().enumerate() {
                                let audio = dishonored2_rust::assets::wwise::decode_wem(&variant.media)?;
                                println!("  event {id:#010x} group {group_index} {:?} variant {variant_index} weight {} -> decoded {} bytes {}", group.mode, variant.weight, audio.len(), String::from_utf8_lossy(&audio[..audio.len().min(4)]));
                            }
                        }
                    }
                    Ok(None) => println!("  event {id:#010x} -> unresolved"),
                }
            }
            continue;
        }
        inspect_bank(&bank, object_filter)?;
    }
    Ok(())
}

fn print_playback(node: &dishonored2_rust::assets::wwise::PlaybackNode, depth: usize,
    mixer: &dishonored2_rust::gameplay::sound::mix::Mixer,
    continuation: Option<(u32, &dishonored2_rust::assets::wwise::ContainerPlayback)>) -> Result<(), Box<dyn Error>> {
    use dishonored2_rust::assets::wwise::{decode_wem_pcm, PlaybackNode};
    let indent = "  ".repeat(depth);
    match node {
        PlaybackNode::Missing { node } => println!("{indent}Unresolved playlist actor {node:#010x}"),
        PlaybackNode::Sound { node, variant } => {
            let audio = decode_wem_pcm(&variant.media)?;
            let loops = dishonored2_rust::gameplay::sound::looping::LoopCount::read(&variant.properties)?;
            println!("{indent}Sound {node:#010x}: {} samples, {} channels, {} Hz, loops={loops:?}, loop_points={:?}",
                audio.samples.len(), audio.channels, audio.sample_rate, audio.loop_points);
            let source = dishonored2_rust::gameplay::sound::looping::SourceLoop::new(audio.loop_points,
                audio.samples.len() / usize::from(audio.channels), loops.count as u16)?;
            println!("{indent}  Authored decoder duration before random loop modifiers/pitch: {} ms (0 = infinite)",
                source.duration_ms(audio.sample_rate));
            if let Some((owner, settings)) = continuation {
                use dishonored2_rust::gameplay::sound::{containers::timing, resampler::pitch_ratio};
                let duration = source.duration_ms(audio.sample_rate)
                    / pitch_ratio(mixer.source_property(&variant.properties, 2, None)?);
                if let Some(plan) = timing::crossfade(settings.transition, duration, settings.transition_ms, 48000) {
                    println!("{indent}  Crossfade if container {owner:#010x} continues, before random modifiers/transition RTPC: {plan:?}");
                }
            }
            let controls = mixer.auxiliary_controls(&variant.properties, &[], &[])?;
            println!("{indent}  Auxiliary buses with no game-object sends: {:?}",
                dishonored2_rust::gameplay::sound::auxiliary::bus_sends(&controls));
            for properties in &variant.properties.nodes {
                println!("{indent}  Node {:#010x} auxiliary_flags={:#x} buses={:x?}",
                    properties.id, properties.auxiliary_flags, properties.auxiliary_buses);
                for curve in &properties.curves {
                    println!("{indent}  Node {:#010x} RTPC={:#010x} property={} kind={} accumulation={} scaling={} points={:?}",
                        properties.id, curve.parameter, curve.property, curve.kind, curve.accumulation, curve.scaling, curve.points);
                }
                if !properties.ranges.is_empty() {
                    println!("{indent}  Node {:#010x} ranges={:?}", properties.id, properties.ranges);
                }
                if properties.float(2) != 0.0 || properties.float(3) != 0.0 || properties.float(4) != 0.0 {
                    println!("{indent}  Node {:#010x} pitch={} LPF={} HPF={}", properties.id,
                        properties.float(2), properties.float(3), properties.float(4));
                }
            }
            for properties in &variant.properties.nodes {
                if properties.positioning_flags & 1 != 0 {
                    println!("{indent}  Positioning node={:#010x} flags={:#x} {:?}", properties.id, properties.positioning_flags, properties.positioning_3d);
                    break;
                }
            }
        },
        PlaybackNode::Layer { node, children, .. } => {
            println!("{indent}Layer {node:#010x}");
            // 14112f0a0 does not schedule an outer early crossfade through
            // a parallel group; its last branch releases that continuation.
            let continuation = if children.len() == 1 { continuation } else { None };
            for child in children { print_playback(child, depth + 1, mixer, continuation)?; }
        },
        PlaybackNode::Container(container) => {
            println!("{indent}Container {:#010x}: {:?}, global={}, reset={}, avoid_repeat={}, {:?}",
                container.node, container.mode, container.global, container.reset_each_play, container.avoid_repeat, container.playback);
            for (child, weight) in &container.children {
                println!("{indent}  weight={weight}");
                let continuation = if container.playback.continuous { Some((container.node, &container.playback)) } else { continuation };
                print_playback(child, depth + 2, mixer, continuation)?;
            }
        },
    }
    Ok(())
}

fn event_ids(bank: &[u8]) -> Result<Vec<u32>, Box<dyn Error>> {
    let mut offset = 0usize;
    while offset + 8 <= bank.len() {
        let size = u32::from_le_bytes(bank[offset + 4..offset + 8].try_into()?) as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or("chunk size overflow")?;
        let chunk = bank.get(start..end).ok_or("chunk outside bank")?;
        if &bank[offset..offset + 4] == b"HIRC" {
            let count = read_u32(chunk, 0)? as usize;
            let mut cursor = 4usize;
            let mut ids = Vec::new();
            for _ in 0..count {
                let kind = *chunk.get(cursor).ok_or("truncated HIRC")?;
                let object_size = read_u32(chunk, cursor + 1)? as usize;
                let object = chunk.get(cursor + 5..cursor + 5 + object_size).ok_or("HIRC object outside chunk")?;
                if kind == 4 { ids.push(read_u32(object, 0)?); }
                cursor += 5 + object_size;
            }
            return Ok(ids);
        }
        offset = end;
    }
    Ok(Vec::new())
}

fn inspect_bank(bank: &[u8], object_filter: Option<u32>) -> Result<(), Box<dyn Error>> {
    let mut offset = 0usize;
    while offset.checked_add(8).is_some_and(|end| end <= bank.len()) {
        let tag = &bank[offset..offset + 4];
        let size = u32::from_le_bytes(bank[offset + 4..offset + 8].try_into()?) as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or("Wwise bank chunk size overflow")?;
        if end > bank.len() {
            return Err("Wwise bank chunk extends beyond its package entry".into());
        }
        println!("  chunk {}: {} bytes", String::from_utf8_lossy(tag), size);
        if tag == b"DIDX" && size % 12 == 0 {
            for record in bank[start..end].chunks_exact(12) {
                println!("    embedded media id={:#x} offset={:#x} size={}", read_u32(record, 0)?, read_u32(record, 4)?, read_u32(record, 8)?);
            }
        } else if tag == b"BKHD" && size >= 8 {
            let version = u32::from_le_bytes(bank[start..start + 4].try_into()?);
            let bank_id = u32::from_le_bytes(bank[start + 4..start + 8].try_into()?);
            println!("    version={version} bank_id={bank_id:#x}");
        } else if tag == b"HIRC" {
            inspect_hierarchy(&bank[start..end], object_filter)?;
        }
        offset = end;
    }
    if offset != bank.len() {
        return Err("trailing bytes after Wwise bank chunks".into());
    }
    Ok(())
}

fn inspect_hierarchy(hierarchy: &[u8], object_filter: Option<u32>) -> Result<(), Box<dyn Error>> {
    let count = read_u32(hierarchy, 0)? as usize;
    let mut offset = 4usize;
    let mut kinds = BTreeMap::<u8, usize>::new();
    for _ in 0..count {
        let kind = *hierarchy.get(offset).ok_or("truncated Wwise hierarchy")?;
        let size = read_u32(hierarchy, offset + 1)? as usize;
        let start = offset + 5;
        let end = start.checked_add(size).ok_or("Wwise hierarchy object size overflow")?;
        let object = hierarchy.get(start..end).ok_or("Wwise hierarchy object outside chunk")?;
        *kinds.entry(kind).or_default() += 1;
        if object_filter.is_some_and(|id| size >= 4 && read_u32(object, 0).ok() == Some(id)) {
            println!("    object type={kind} id={:#x} raw={:02x?}", read_u32(object, 0)?, &object[..object.len().min(128)]);
        } else if object_filter.is_none() && matches!(kind, 5 | 9) && size >= 4 {
            println!("    object type={kind} id={:#x} raw={:02x?}", read_u32(object, 0)?, &object[..object.len().min(96)]);
        } else if object_filter.is_none() && kind == 2 && size >= 4 {
            println!("    sound {} raw={:02x?}", read_u32(object, 0)?, &object[..object.len().min(40)]);
        } else if object_filter.is_none() && kind == 4 && size >= 5 {
            let id = read_u32(object, 0)?;
            let action_count = object[4] as usize;
            println!("    event {id:#x} raw={:02x?}", &object[..object.len().min(24)]);
            if size >= 8 + action_count * 4 {
                let actions = (0..action_count).map(|index| read_u32(object, 8 + index * 4))
                    .collect::<Result<Vec<_>, _>>()?;
                println!("    event {id:#x} actions={actions:#x?}");
            }
        } else if object_filter.is_none() && kind == 3 && size >= 5 {
            println!("    action {} payload={:02x?}", read_u32(object, 0)?, &object[4..object.len().min(20)]);
        }
        offset = end;
    }
    if offset != hierarchy.len() {
        return Err("Wwise hierarchy object count did not consume its chunk".into());
    }
    println!("    HIRC objects: {kinds:?}");
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Box<dyn Error>> {
    Ok(u32::from_le_bytes(bytes.get(offset..offset + 4).ok_or("truncated Wwise bank field")?.try_into()?))
}

use super::*;
use crate::assets::{archive::Catalog, entities, installation};
use crate::gameplay::effects::environment::EnvironmentBlendEffect;

#[test]
fn group_defaults_match_executed_reset() {
    let group = environment_group(&[]).unwrap();
    let mut actual = vec![u32::from(group.enabled), group.location as u32];
    actual.extend(
        [
            group.timings.delay,
            group.timings.fade_in,
            group.timings.duration,
            group.timings.fade_out,
        ]
        .into_iter()
        .flatten()
        .map(f32::to_bits),
    );
    actual.push(group.environments.len() as u32);
    let expected = include_str!("../../../tests/fixtures/effect_group_defaults.txt")
        .split_whitespace()
        .map(|w| u32::from_str_radix(w, 16).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn parent_group_ranges_and_conditions_preserve_partial_edits() {
    let fields = entities::parse_declaration(
        r#"{
        m_conditions = { m_location = "OUT_OF_WATER"; }
        m_timings = { m_delay = { m_min = 0.5; } m_duration = { m_max = 0; } }
        m_envOverrides = { num = 2; item[1] = { m_enabled = false; } }
    }"#,
    )
    .unwrap();
    let group = environment_group(&fields).unwrap();
    assert_eq!(group.location, EffectGroupLocation::OutOfWater);
    assert_eq!(group.timings.delay, [0., 0.5]);
    assert_eq!(group.timings.duration, [0., 1.]);
    assert_eq!(group.environments.len(), 2);
    assert!(group.environments[0].enabled);
    assert!(!group.environments[1].enabled);
    assert!(group.environments.iter().all(|e| e.environment.is_none()));
}

#[test]
fn float_list_resize_matches_executed_reader() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/effect_float_list_resize.txt")
        .lines()
        .enumerate()
    {
        let words = row
            .split_whitespace()
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect::<Vec<_>>();
        let n = words[0] as usize;
        let m = words[1] as usize;
        assert_eq!(words.len(), 2 + n + m);
        let mut values = words[2..2 + n]
            .iter()
            .map(|w| f32::from_bits(*w))
            .collect::<Vec<_>>();
        float_list(&mut values, &[("num".into(), Value::Atom(m.to_string()))]).unwrap();
        assert_eq!(
            values.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
            words[2 + n..],
            "case {case}"
        );
        count += 1;
    }
    assert_eq!(count, 512);
}

#[test]
fn sparse_environment_edits_retain_constructor_curves() {
    let fields = entities::parse_declaration(
        r#"{
        m_name = "sample"; m_decl = "env/sample";
        m_timings = { m_looping = true; m_finishLoopOnStop = true;
            m_duration = { m_max = 4; m_min = 2; } }
        m_blend = { m_min = 0.5; m_fadeinCurve = { m_maxCurve = {
            m_values = { item[0] = -0.25; item[2] = 0.25; }
        } } }
    }"#,
    )
    .unwrap();
    let effect = environment_effect(&fields).unwrap();
    assert_eq!(effect.name, "sample");
    assert_eq!(effect.environment.as_deref(), Some("env/sample"));
    assert!(effect.enabled && effect.blend.looping && effect.blend.keep_duration_on_stop);
    assert_eq!(effect.blend.timing.duration, [4., 2.]);
    assert_eq!(effect.blend.scale, [1., 0.5]);
    assert_eq!(effect.blend.duration.maximum.times, [0.]);
    assert_eq!(effect.blend.duration.maximum.values, [0., 1., 0.]);
    assert_eq!(effect.blend.fade_in.maximum.times, [0., 1.]);
    assert_eq!(
        effect.blend.fade_in.maximum.values,
        [-0.25, 0., 0.25, 0., 1., 0.]
    );
    assert!(effect.blend.fade_in.minimum.times.is_empty());
}

#[test]
fn expanded_curve_arrays_zero_new_entries() {
    let fields = entities::parse_declaration(
        r#"{
        m_blend = { m_durationCurve = { m_maxCurve = {
            m_times = { num = 2; }
        } } }
    }"#,
    )
    .unwrap();
    assert_eq!(
        environment_effect(&fields)
            .unwrap()
            .blend
            .duration
            .maximum
            .times,
        [0., 0.]
    );
}

#[test]
#[ignore = "requires installed FX declarations"]
fn installed_environment_effect_descriptors_load_and_evaluate() {
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    let mut paths = catalog.paths_with_suffix(".decl");
    paths.sort();
    let mut files = 0;
    let mut effects = 0;
    let mut running = 0;
    let mut seed = 21_240;
    let field = |fields: &Fields, name: &str| {
        fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    for path in paths
        .into_iter()
        .filter(|p| p.starts_with("generated/decls/fx/"))
    {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let fields = entities::parse_declaration(&entities::declaration_text(&bytes))
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        files += 1;
        let Some(edit) = field(&fields, "edit") else {
            continue;
        };
        let Some(groups) = field(scope(&edit).unwrap(), "m_groups") else {
            continue;
        };
        for (group_name, group) in scope(&groups)
            .unwrap()
            .iter()
            .filter(|(n, _)| n.starts_with("item["))
        {
            let group = environment_group(scope(group).unwrap())
                .unwrap_or_else(|e| panic!("{path}/{group_name}: {e}"));
            for (index, asset) in group.environments.into_iter().enumerate() {
                let label = format!("{path}/{group_name}/item[{index}]");
                effects += 1;
                if asset.enabled && asset.environment.is_some() {
                    let resource = format!(
                        "generated/decls/env/{}.decl",
                        asset.environment.as_ref().unwrap()
                    );
                    assert!(
                        catalog.contains(&resource),
                        "{label}: missing environment {resource}"
                    );
                    let mut effect =
                        EnvironmentBlendEffect::new(asset.blend, [0.; 4], 1, &mut seed)
                            .unwrap_or_else(|e| panic!("{label}: {e}"));
                    for age in [0., 0.125, 0.5, 1., 3., 10.] {
                        effect
                            .update(age, 1, &mut seed)
                            .unwrap_or_else(|e| panic!("{label}: {e}"));
                    }
                    running += 1;
                }
            }
        }
    }
    assert!(files > 0 && effects > 0 && running > 0);
    println!("Loaded {effects} environment effect descriptors from {files} FX files; evaluated {running} eligible descriptors");
}

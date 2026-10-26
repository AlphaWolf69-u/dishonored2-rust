use super::*;
#[test]
fn trigger_bindings_preserve_component_forms_overlays_and_owner_flags() {
    let document=super::super::parse(r#"Version 6
component { cpntTrigger shared { inherit="parent.cpnttrigger";edit={m_triggeredByDebris=true;m_isKeyframed=true;} } }
component { cpntWater water {} }
entity { entityDef ordinary {class="idEntity";edit={m_componentDecls={
item_add["trigger"]={mapComponentDecl="shared";inlineDecl={m_triggeredByNpcCapsules=false;}}
item_add["wind"]={inheritedDecl="disabled.cpntwind";}}}} }
entity { entityDef pool {class="arkAreaVolume";edit={m_componentDecls={
item_add["water"]={mapComponentDecl="water";}
item_add["trigger"]={inheritedDecl="default.cpnttrigger";}
item_add["second"]={inheritedDecl="default.cpnttrigger";inlineDecl={m_triggeredByRigidBodies=true;}}}}} }
"#).unwrap();
    let result = trigger_bindings(&document, |path| match path {
        "generated/decls/cpnttrigger/parent.cpnttrigger.decl" => {
            Ok(Some("{m_triggeredByNpcRagdolls=true;}".into()))
        }
        "generated/decls/cpnttrigger/default.cpnttrigger.decl" => Ok(Some("{}".into())),
        _ => panic!("unexpected declaration {path}"),
    })
    .unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].entity_id, "ordinary");
    assert_eq!(result[0].slot, "item_add[\"trigger\"]");
    let first = &result[0];
    assert!(first.collision.debris && first.collision.npc_ragdolls && first.collision.keyframed);
    assert!(
        !first.collision.rigid_bodies && !first.collision.npc_capsules && !first.water_component
    );
    assert!(first.wind_component);
    assert_eq!(result[1].entity_id, "pool");
    assert!(result[1].water_component && !result[1].wind_component);
    assert!(
        result[1].collision.npc_capsules
            && !result[1].collision.npc_ragdolls
            && !result[1].collision.keyframed
    );
    assert!(result[2].collision.rigid_bodies);
}

#[test]
fn trigger_bindings_reject_unresolved_declarations() {
    let document=super::super::parse(r#"Version 6
entity { entityDef broken {edit={m_componentDecls={item_add["trigger"]={mapComponentDecl="missing";}}}} }
"#).unwrap();
    assert!(trigger_bindings(&document, |_| panic!(
        "unresolved map reference must fail before archive access"
    ))
    .is_err());
}

#[test]
fn trigger_placements_apply_factory_pose_and_resource_rules() {
    let document = super::super::parse(
        r#"Version 6
entity { entityDef generated { edit={
 m_componentDecls={item_add["t"]={inheritedDecl="cpnttrigger_default.cpnttrigger";}}
 spawnPosition={x=2;y=3;z=4;}
 spawnOrientation={mat={mat[0]={x=0;y=1;z=0;} mat[1]={x=-1;y=0;z=0;}}}
}} }
entity { entityDef model { edit={
 m_componentDecls={item_add["t"]={inheritedDecl="cpnttrigger_default.cpnttrigger";}}
 renderModelInfo={model="models/trigger.lwo";scale={x=5;y=5;z=5;}}
 spawnOrientation={mat={mat[0]={x=0;y=1;z=0;} mat[1]={x=-1;y=0;z=0;}}}
}} }
"#,
    )
    .unwrap();
    let placements =
        trigger_placements(&document, "campaign/test.map", |_| Ok(Some("{}".into()))).unwrap();
    assert_eq!(placements[0].position, [2., 3., 4.]);
    assert_eq!(placements[0].shape.name, "maps/campaign/test/generated");
    assert_eq!(
        placements[0].shape.basis,
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
    );
    assert_eq!(placements[1].position, [0.; 3]);
    assert_eq!(
        placements[1].shape.basis,
        [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]]
    );
    assert_eq!(
        placements[1].shape.asset_path(),
        "generated/models/trigger.lwo.cvd._100_100_100.0.hkx"
    );
}

#[test]
#[ignore = "requires installed level and physics archives"]
fn installed_trigger_shape_requests_resolve_every_body_resource() {
    use crate::{archive::Catalog, assets::level::LevelResources};
    use std::collections::{BTreeMap, BTreeSet};
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let paths: BTreeSet<_> = catalog
        .paths_with_suffix(".bwm")
        .into_iter()
        .map(|path| LevelResources::resolve(&catalog, &path).unwrap().entities)
        .collect();
    let mut roots = BTreeMap::<String, usize>::new();
    let mut worlds = 0;
    for path in paths {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let document = super::super::parse(&super::super::declaration_text(&bytes)).unwrap();
        let map = format!(
            "{}.map",
            path.strip_prefix("maps/")
                .unwrap()
                .strip_suffix(".entities")
                .unwrap()
        );
        let assets = TriggerAssets::load(&document, &map, &catalog)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(assets.placements.len(), assets.shape_resources.len());
        for (placement, index) in assets.placements.iter().zip(&assets.shape_resources) {
            let asset = &assets.shapes[*index];
            assert_eq!(placement.shape.asset_path(), asset.path);
            let root = &asset.physics.types[asset.physics.items[1].type_index].name;
            *roots.entry(root.clone()).or_default() += 1;
        }
        for asset in &assets.shapes {
            let _compound = asset
                .physics
                .compound_shape(&asset.bytes, 1)
                .unwrap_or_else(|e| panic!("{}: {e}", asset.path));
        }
        worlds += 1;
    }
    assert!(worlds > 0 && !roots.is_empty());
    println!("Installed trigger worlds={worlds}; root shape types={roots:?}");
}

#[test]
#[ignore = "requires installed level and component archives"]
fn installed_trigger_bindings_cover_environment_volumes() {
    use crate::{archive::Catalog, assets::level::LevelResources};
    use std::collections::BTreeSet;
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let paths: BTreeSet<_> = catalog
        .paths_with_suffix(".bwm")
        .into_iter()
        .map(|path| LevelResources::resolve(&catalog, &path).unwrap().entities)
        .collect();
    let mut counts = [0usize; 6];
    for path in paths {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let document = super::super::parse(&super::super::declaration_text(&bytes)).unwrap();
        let triggers = trigger_bindings(&document, |name| {
            catalog
                .read(name)
                .map(|bytes| bytes.map(|bytes| super::super::declaration_text(&bytes).into_owned()))
                .map_err(|e| e.to_string())
        })
        .unwrap_or_else(|e| panic!("{path}: {e}"));
        let areas = super::super::placed_environment_volumes(&document);
        for area in &areas {
            assert!(
                triggers.iter().any(|trigger| trigger.entity_id == area.id),
                "{path}: {}",
                area.id
            );
        }
        let identities: BTreeSet<_> = triggers
            .iter()
            .map(|trigger| (&trigger.entity_id, &trigger.slot))
            .collect();
        assert_eq!(
            identities.len(),
            triggers.len(),
            "{path}: repeated component identity"
        );
        counts[0] += 1;
        counts[1] += triggers.len();
        counts[2] += areas.len();
        counts[3] += triggers.iter().filter(|t| t.water_component).count();
        counts[4] += triggers.iter().filter(|t| t.wind_component).count();
        counts[5] += triggers.iter().filter(|t| t.collision.keyframed).count();
    }
    assert!(counts[0] > 0 && counts[1] > 0 && counts[2] > 0);
    println!("Installed worlds/triggers/areas/water/wind/keyframed={counts:?}");
}

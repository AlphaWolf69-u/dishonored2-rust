//! Owned ammo variants:637e40 swaps render model/materials and63b5f0
//! replaces an existing component declaration before its normal registration.
//! Base entity prototypes remain immutable in their canonical registry.
use super::*;
use crate::assets::equipment::attachment::AmmoUpgrade;

pub(crate) fn weapon_variant(
    catalog: &Catalog,
    base: Arc<Definition>,
    upgrades: &[AmmoUpgrade],
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<Arc<Definition>, String> {
    if upgrades.iter().all(|upgrade| {
        upgrade.projectile.is_none()
            && upgrade.grenade.is_none()
            && upgrade.mine.is_none()
            && upgrade.static_model.is_none()
            && upgrade.materials.is_empty()
    }) {
        return Ok(base);
    }
    active(cancelled)?;
    let mut variant = (*base).clone();
    let mut replaced = [false; 3];
    for upgrade in upgrades {
        for (family, override_decl) in [&upgrade.projectile, &upgrade.grenade, &upgrade.mine]
            .into_iter()
            .enumerate()
        {
            let Some(override_decl) = override_decl else {
                continue;
            };
            if replaced[family] {
                return Err(format!(
                    "{}: multiple ammo overrides for component family {family}",
                    base.entity
                ));
            }
            replaced[family] = true;
            let kind = ["cpntprojectile", "cpntgrenade", "cpntmine"][family];
            let component = variant
                .components
                .iter_mut()
                .find(|component| match (&component.kind, family) {
                    (Kind::Projectile(_), 0) => true,
                    _ => false,
                })
                .ok_or_else(|| {
                    format!(
                        "{}: ammo override {override_decl} requires actual {kind} component",
                        base.entity
                    )
                })?;
            //63b5f0 rejects replacing an inline declaration or changing the
            //existing class. All component slots/factory order stay intact.
            if component.inline {
                return Err(format!(
                    "{}: cannot override inline component {}",
                    base.entity, component.slot
                ));
            }
            let entry = vec![(
                "inheritedDecl".into(),
                entities::Value::Atom(override_decl.clone()),
            )];
            let mut source_paths = Vec::new();
            let declaration_kind = catalog.component_declaration_kind(kind)?.to_ascii_lowercase();
            let fields = entities::components::entry_fields(
                &entry,
                &declaration_kind,
                &HashMap::new(),
                &mut HashMap::new(),
                &mut |path| {
                    active(cancelled)?;
                    source_paths.push(path.to_owned());
                    super::super::declaration_texts::read_common_or_catalog(catalog, path)
                },
            )?
            .ok_or_else(|| {
                format!("ammo override {override_decl} changes component class {kind}")
            })?;
            match family {
                0 => {
                    let projectile = super::super::projectiles::Declaration::from_resolved(
                        catalog,
                        &base.entity,
                        &fields,
                    )?;
                    component.kind = Kind::Projectile(projectile.clone());
                    variant.projectile = Some(projectile);
                }
                _ => unreachable!("actual missing component family was rejected above"),
            }
            component.declaration = override_decl.clone();
            component.scheduling = Arc::new(entities::components::scheduling(
                &fields,
                DeclarationScheduling::from_base_flags(0),
            )?);
            variant.source_paths.extend(source_paths);
        }
    }
    if upgrades
        .iter()
        .any(|upgrade| upgrade.static_model.is_some() || !upgrade.materials.is_empty())
    {
        variant.model = Some(super::super::equipment::attachment::ammo_model_variant(
            catalog,
            base.model
                .clone()
                .ok_or("ammo model override has no actual render model")?,
            upgrades,
            cancelled,
        )?);
    }
    super::stick::rebuild(catalog, &mut variant, cancelled)?;
    variant.source_paths.sort_unstable();
    variant.source_paths.dedup();
    active(cancelled)?;
    Ok(Arc::new(variant))
}

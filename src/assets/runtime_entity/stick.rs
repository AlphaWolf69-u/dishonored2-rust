//!627f60's authored post-stick component/model resources. These are immutable
//! prototype dependencies, not actors attached before the actual stick event.
use super::*;
pub(super) struct Prepared {
    pub components: Vec<Component>,
    pub model: Option<Arc<super::super::equipment::attachment::Descriptor>>,
    pub chunks: Vec<Chunk>,
    pub source_paths: Vec<String>,
}
pub(super) fn prepare(
    catalog: &Catalog,
    entity: &str,
    class: &str,
    types: &ComponentTypes,
    projectile: Option<&super::super::projectiles::Declaration>,
    base_model: Option<Arc<super::super::equipment::attachment::Descriptor>>,
    cancelled: Option<&dyn Fn() -> bool>,
    stack: &mut Vec<String>,
) -> Result<Prepared, String> {
    let mut result = Prepared {
        components: Vec::new(),
        model: None,
        chunks: Vec::new(),
        source_paths: Vec::new(),
    };
    let Some(projectile) = projectile else {
        return Ok(result);
    };
    let mut requires_animation_tree = false;
    for (index, declaration) in projectile.stick.components_to_attach.iter().enumerate() {
        let acquired = Component::acquire_decl(
            catalog,
            types,
            entity,
            &format!("m_componentsToAttach[{index}]"),
            declaration,
            cancelled,
        )?;
        requires_animation_tree |= acquired.decoded.animation.is_some();
        if let Kind::Breakable(definition) = &acquired.decoded.component.kind {
            for (step, chunk, child) in definition.chunks() {
                result.chunks.push(Chunk {
                    component: index,
                    step,
                    chunk,
                    definition: Definition::acquire_inner(catalog, &name(child), cancelled, stack)?,
                });
            }
        }
        result.source_paths.extend(acquired.source_paths);
        result.components.push(acquired.decoded.component);
    }
    result.model = projectile
        .stick
        .model_change
        .as_deref()
        .map(|model| {
            super::super::equipment::attachment::static_model_variant(
                catalog,
                entity,
                class,
                base_model.clone(),
                model,
                cancelled,
            )
        })
        .transpose()?;
    //53e610 leaves AnimBase inert without a real skeleton. A declared tree
    //alone is not an unsupported-family error for a rigid/model-less actor.
    if requires_animation_tree
        && result
            .model
            .as_ref()
            .or(base_model.as_ref())
            .is_some_and(|model| {
                matches!(
                    &model.geometry,
                    super::super::equipment::attachment::Geometry::Animated { .. }
                )
            })
    {
        return Err(format!(
            "{entity}: attached AnimBase tree requires its actual animator resource owner"
        ));
    }
    Ok(result)
}
pub(super) fn rebuild(
    catalog: &Catalog,
    definition: &mut Definition,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<(), String> {
    let prepared = prepare(
        catalog,
        &definition.entity,
        &definition.class,
        &definition.types,
        definition.projectile.as_deref(),
        definition.model.clone(),
        cancelled,
        &mut Vec::new(),
    )?;
    validate_capabilities(
        &definition.types,
        definition
            .components
            .iter()
            .chain(prepared.components.iter()),
    )?;
    definition.stick_components = prepared.components;
    definition.stick_model = prepared.model;
    definition.stick_chunks = prepared.chunks;
    definition.source_paths.extend(prepared.source_paths);
    Ok(())
}

//! 752900/878fd0 binding admission for actual admitted scene actors.
//! The renderer owns sampled joint frames; this owner retains identities and
//! local offsets only, never another skin, clip, physics world or model cache.
use super::*;
use crate::gameplay::{entity_lifetime::LiveEntities, reference_frame::{ReferenceFrame, relative_binding, compose}};

#[derive(Default)]
pub(super) struct DocumentHierarchy {
    pub(super) bindings: Vec<Binding>,
    /// Initial authored world frames for actual base entity ancestors.
    pub(super) ancestors: HashMap<usize, (ReferenceFrame, Option<usize>)>,
    render_frames: HashMap<usize, ReferenceFrame>,
    /// Sparse actual player/externally published joints requested by live binds.
    pub(super) external_joints: HashMap<(usize,i16),ReferenceFrame>,
}

pub(super) struct Binding {
    pub(super) child: usize,
    pub(super) parent: usize,
    pub(super) parent_placement: Option<usize>,
    pub(super) joint: i16,
    child_actor: (ComponentHandle, u32),
    parent_actor: (ComponentHandle, u32),
    initial_render_frame: ReferenceFrame,
    last_render_frame: Option<ReferenceFrame>,
}

impl DocumentHierarchy {
    /// Retire per-actor projections with the actual header generation. A
    ///reused entity index must not inherit another actor's render/joint frame.
    pub(super) fn retire_entity_frames(&mut self,entity:usize)->Result<(),String>{
        if self.bindings.iter().any(|binding|binding.child==entity||binding.parent==entity){
            return Err("retiring entity still has document bindings".into());
        }
        self.ancestors.remove(&entity);
        self.render_frames.remove(&entity);
        self.external_joints.retain(|(owner,_),_|*owner!=entity);
        Ok(())
    }
    pub(super) fn record_runtime_binding(&mut self,child:usize,parent:usize,parent_placement:Option<usize>,joint:i16,
        child_actor:(ComponentHandle,u32),parent_actor:(ComponentHandle,u32),initial:ReferenceFrame,parent_frame:ReferenceFrame) {
        self.ancestors.entry(parent).or_insert((parent_frame,parent_placement));
        self.render_frames.entry(parent).or_insert(parent_frame);
        self.render_frames.entry(child).or_insert(initial);
        self.bindings.push(Binding {child,parent,parent_placement,joint,child_actor,parent_actor,initial_render_frame:initial,last_render_frame:None});
    }
}

///The same parent-first closure is used by joint preparation and actual actor
///admission. A binding added by one concrete component family must not miss
///its parent's tag/joint demand merely because it is not a readable.
fn ordered_bindings<'a>(
    authored: &HashMap<String, entities::EntityBind>,
    seeds: impl Iterator<Item = &'a str>,
) -> Result<Vec<String>, String> {
    let mut requested = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    for seed in seeds {
        let mut path = Vec::new();
        let mut next = seed;
        while let Some(binding) = authored.get(next) {
            if path.iter().any(|name: &String| name == next) {
                return Err("cyclic entity binding".into());
            }
            path.push(next.to_owned());
            next = &binding.parent;
        }
        for name in path.into_iter().rev() {
            if requested.insert(name.clone()) {
                ordered.push(name);
            }
        }
    }
    Ok(ordered)
}

///878470 tests the actual parent's model type, independently of visibility.
///A hidden model can be absent from render placements while still owning an
///MD6 skeleton; resolve its authored descriptor rather than inventing a root.
fn binding_model_path<'a>(catalog:&archive::Catalog,definition:&entities::Definition,
    projection:Option<&'a entities::PlacedModel>)->Result<Option<std::borrow::Cow<'a,str>>,String>{
    if let Some(model)=projection{return Ok(Some(std::borrow::Cow::Borrowed(&model.resource_path)));}
    Ok(entities::model_descriptor(definition,|path|crate::assets::declaration_texts::read_common_or_catalog(catalog,path))?
        .map(|model|std::borrow::Cow::Owned(model.resource_path)))
}

pub(in crate::app) fn joint_requests(
    document: &entities::Document,
    interactions: Option<&super::super::interactions::Interactions>,
    models: &[entities::PlacedModel],
) -> Result<Vec<(usize, String, Option<String>)>, String> {
    let Some(interactions) = interactions else {
        return Ok(Vec::new());
    };
    let authored = entities::resolved_entity_binds(document)
        .into_iter()
        .map(|binding| (binding.entity_id.clone(), binding))
        .collect::<HashMap<_, _>>();
    let placements = models
        .iter()
        .enumerate()
        .map(|(index, model)| (model.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let declarations = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "component")
        .map(|definition| (definition.id.as_str(), definition))
        .collect::<HashMap<_, _>>();
    let definitions=document.definitions.iter().filter(|definition|definition.kind=="entity")
        .map(|definition|(definition.id.as_str(),definition)).collect::<HashMap<_,_>>();
    let mut seeds = interactions
        .readable
        .iter()
        .map(|readable| readable.entity.as_str())
        .chain(
            interactions
                .actors
                .iter()
                .map(|actor| actor.entity.as_str()),
        )
        .collect::<Vec<_>>();
    //These are precisely the source families which allocate additional Scene
    //headers before prepare_document_hierarchy: document rigid/Throwable,
    //audio, and trigger placements (including their water/wind components).
    //Read class identity only; this does not decode or retain a second asset.
    for definition in document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "entity")
    {
        let entries = entities::scene_motion::child(
            entities::scene_motion::child(&definition.fields, "edit"),
            "m_componentDecls",
        );
        for (_, value) in entries
            .iter()
            .filter(|(slot, _)| slot.starts_with("item_add["))
        {
            let Some(entry) = entities::scope(Some(value)) else {
                continue;
            };
            if matches!(
                entities::components::entry_class(entry, &declarations)?.as_deref(),
                Some("cpntrigidbody" | "cpntthrowable" | "cpntrigidbodythrowable" | "cpntaudio" | "cpnttrigger")
            ) {
                seeds.push(definition.id.as_str());
                break;
            }
        }
    }
    let mut requests = Vec::new();
    for name in ordered_bindings(&authored, seeds.into_iter())? {
        let binding = &authored[&name];
        if let Some(tag) = &binding.tag {
            let parent=definitions.get(binding.parent.as_str()).ok_or("bound parent source absent")?;
            let model=binding_model_path(&interactions.catalog,parent,placements.get(binding.parent.as_str()).map(|&index|&models[index]))?;
            if model.as_deref().and_then(crate::bmd6::declaration_path).is_none(){continue;}
            let placement = *placements
                .get(binding.parent.as_str())
                .ok_or_else(||format!("hidden MD6 bound parent has no prepared pose owner: {}",binding.parent))?;
            // 879890 selects the parent md6 property group by the bound
            // child's model; a child without one uses _info.
            let child=definitions.get(binding.entity_id.as_str()).ok_or("bound child source absent")?;
            let group=binding_model_path(&interactions.catalog,child,placements.get(binding.entity_id.as_str()).map(|&index|&models[index]))?
                .map(std::borrow::Cow::into_owned);
            if !requests
                .iter()
                .any(|(index, name, old)| *index == placement && name == tag && *old == group)
            {
                requests.push((placement, tag.clone(), group));
            }
        }
    }
    Ok(requests)
}

impl PhysicsScene {
    pub(in crate::app) fn bound_render_entities(&self) -> impl Iterator<Item = &str> {
        self.document_hierarchy.bindings.iter()
            .filter(|binding| self.rigid_placements.contains_key(&binding.child))
            .map(|binding| self.entities[binding.child].name.as_str())
    }

    pub(super) fn validate_document_hierarchy(&self, live: &LiveEntities, child: usize) -> Result<(), String> {
        let entity = self.entities.get(child).ok_or("bound document entity absent")?;
        let expected = self.document_hierarchy.bindings.iter().find(|binding| binding.child == child);
        let parent = live.parent(entity.shared)?;
        if parent != expected.map(|binding| self.entities[binding.parent].shared) {
            return Err("document entity and actor attachment owners differ".into());
        }
        if let Some(binding) = expected {
            let actor = entity.components.reference_actor.and_then(|handle| self.actors.actors.get(handle.slot))
                .ok_or("bound document actor absent")?;
            if actor.binding.entity != Some(binding.parent) || actor.binding.joint != binding.joint {
                return Err("document binding differs from its live actor".into());
            }
        }
        for &id in live.children(entity.shared)? {
            if !self.document_hierarchy.bindings.iter().any(|binding| binding.parent == child && self.entities[binding.child].shared == id) {
                return Err("document descendant has no admitted attachment owner".into());
            }
        }
        Ok(())
    }

    /// 727a00 walks the live parent physics interfaces, using the first
    /// started interface's actual movement type. No active interface means
    /// keyframed; an unbound component keeps its declaration's own type.
    pub(super) fn bound_document_behavior(&self, live: &LiveEntities, child: usize, authored: u8) -> Result<u8, String> {
        let mut next = live.parent(self.entities[child].shared)?;
        if next.is_none() { return Ok(authored); }
        while let Some(id) = next {
            let &index = self.shared_indices.get(&id).ok_or("bound parent identity is not admitted")?;
            for &packed in self.entities[index].components.entries.values() {
                let handle = ComponentHandle::from_packed(packed)?;
                if !self.is_rigid_class(handle.class_id) { continue; }
                let component = self.rigid_component(handle)?;
                if component.owner.flags & 0xc < 8 { continue; }
                let world = self.world.read().map_err(|_| "level physics query owner is poisoned")?;
                let body = world.storage.bodies.get(component.body().ok_or("bound parent body absent")?).ok_or("bound parent body retired")?;
                return Ok(if body.flags & 1 != 0 { 0 } else if body.flags & 4 != 0 { 1 } else { 2 });
            }
            next = live.parent(id)?;
        }
        Ok(1)
    }

    /// All identities are allocated from real document definitions before the
    /// normal initialized/spawned pass. No unrelated gameplay factory runs.
    pub(super) fn prepare_document_hierarchy(&mut self, document: &entities::Document,
        interactions: &super::super::interactions::Interactions, models: &[entities::PlacedModel],
        live: &mut LiveEntities, registry: &mut ComponentRegistry,
        constructor_tag: crate::gameplay::name_pool::NameId,
        joint_frames: &super::super::joint_frames::JointFrames) -> Result<(), String> {
        let authored = entities::resolved_entity_binds(document).into_iter()
            .map(|binding| (binding.entity_id.clone(), binding)).collect::<HashMap<_, _>>();
        let definitions = document.definitions.iter().filter(|entry| entry.kind == "entity")
            .map(|entry| (entry.id.as_str(), entry)).collect::<HashMap<_, _>>();
        let classes = self.types.classes().hierarchy().registration_order().iter().map(|&index| {
            let class = self.types.classes().metadata(index).ok_or("bound ancestor class metadata absent")?;
            Ok((self.types.classes().name_text(class.name).ok_or("bound ancestor class name absent")?.to_vec(), index))
        }).collect::<Result<HashMap<_, _>, String>>()?;
        let mut indices = self.entities.iter().enumerate().map(|(index, entity)| (entity.name.clone(), index))
            .collect::<HashMap<_, _>>();
        let placements = models.iter().enumerate().map(|(index, model)| (model.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let ordered = ordered_bindings(&authored, self.entities.iter().map(|entity| entity.name.as_str()))?;
        for name in ordered {
            let binding = &authored[&name];
            for name in [&binding.parent, &binding.entity_id] {
                if indices.contains_key(name) { continue; }
                let definition = definitions.get(name.as_str()).ok_or_else(|| format!("bound ancestor declaration absent: {name}"))?;
                let class = definition.fields.iter().find_map(|(key, value)| match (key.as_str(), value) {
                    ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')), _ => None,
                }).ok_or("bound ancestor class absent")?;
                let class = *classes.get(class.as_bytes()).ok_or_else(|| format!("bound ancestor class is unregistered: {class}"))?;
                let pose = entities::spawn_frame(definition)?;
                let index = self.entities.len();
                let shared = live.allocate_managed(Arc::from(name.as_str()), class, None, 0, constructor_tag)?;
                self.entities.push(PhysicsEntity { retired: false, external: false, name: name.clone(), shared, components: EntityComponents::new(index),
                    class, environment: None, flags: 0, entity_flags: 0x001e_8008, entity_flags_d7: 0, priority: 0,
                    frame: crate::gameplay::reference_frame::PublishedFrame { pose,
                        ..crate::gameplay::reference_frame::ReferenceFrameCache::default().published() }, timespace: 0 });
                self.entity_flags_d4.push(0);
                self.document_hierarchy.ancestors.insert(index, (pose, placements.get(name.as_str()).copied()));
                indices.insert(name.clone(), index);
            }
            let child = indices[&binding.entity_id];
            let parent = indices[&binding.parent];
            let initial_render_frame = self.entities[child].frame.pose;
            self.document_hierarchy.ancestors.entry(parent).or_insert((self.entities[parent].frame.pose, placements.get(binding.parent.as_str()).copied()));
            self.document_hierarchy.render_frames.insert(parent, self.entities[parent].frame.pose);
            self.document_hierarchy.render_frames.insert(child, self.entities[child].frame.pose);
            let placement = placements.get(binding.parent.as_str()).copied();
            //752900 bypasses tag/bone lookup when878470 rejects type2.
            let skeletal=if binding.tag.is_some(){
                let parent_source=definitions.get(binding.parent.as_str()).ok_or("bound parent source absent")?;
                binding_model_path(&interactions.catalog,parent_source,placement.map(|index|&models[index]))?
                    .as_deref().and_then(crate::bmd6::declaration_path).is_some()
            }else{false};
            let (joint, offset, orientation) = if let Some(tag) = binding.tag.as_ref().filter(|_|skeletal) {
                let placement = placement.ok_or_else(||format!("hidden MD6 bound parent has no prepared pose owner: {}",binding.parent))?;
                let child_source=definitions.get(binding.entity_id.as_str()).ok_or("bound child source absent")?;
                let group=binding_model_path(&interactions.catalog,child_source,placements.get(binding.entity_id.as_str()).map(|&index|&models[index]))?;
                let selected = joint_frames.resolve(placement, group.as_deref(), tag)?.ok_or_else(|| format!("bound parent joint absent: {}:{tag}", binding.parent))?;
                if selected.joint < 0 {
                    //752900 retains -1 when the actual tag and bone are both
                    //absent, then753182 computes ordinary parent-local binding.
                    let (offset, orientation) = relative_binding(self.entities[parent].frame.pose, self.entities[child].frame.pose);
                    (-1, offset, orientation)
                } else if selected.from_tag {
                    self.entities[child].frame.pose = joint_frames.bound_frame(placement, selected.joint, 0,
                        selected.offset, selected.orientation, self.entities[parent].frame.pose)?
                        .ok_or("prepared tag frame is absent")?;
                    (selected.joint, selected.offset, selected.orientation)
                } else {
                    let local = joint_frames.frame(placement, selected.joint, 0)?.ok_or("prepared joint frame is absent")?;
                    let world = compose(self.entities[parent].frame.pose, local);
                    let (offset, orientation) = relative_binding(world, self.entities[child].frame.pose);
                    (selected.joint, offset, orientation)
                }
            } else {
                let (offset, orientation) = relative_binding(self.entities[parent].frame.pose, self.entities[child].frame.pose);
                (-1, offset, orientation)
            };
            let child_handle = self.reference_actor(child, registry)?;
            let parent_handle = self.reference_actor(parent, registry)?;
            let child_actor = child_handle.slot as usize;
            let parent_actor = parent_handle.slot as usize;
            let child_generation = self.actors.actors[child_actor].owner.generation;
            let parent_generation = self.actors.actors[parent_actor].owner.generation;
            self.actors.actors[parent_actor].children.try_reserve(1).map_err(|error| format!("bound actor child list: {error}"))?;
            self.document_hierarchy.bindings.try_reserve(1).map_err(|error| format!("bound document roster: {error}"))?;
            live.link_child(self.entities[parent].shared, self.entities[child].shared)?;
            self.actors.actors[parent_actor].children.push(child_actor);
            let actor = &mut self.actors.actors[child_actor];
            actor.parent = Some(parent_actor);
            actor.binding.entity = Some(parent);
            actor.binding.joint = joint;
            actor.binding.offset = offset;
            actor.binding.orientation = orientation;
            actor.flags |= 0x10;
            self.entities[child].entity_flags |= 0x80;
            self.document_hierarchy.bindings.push(Binding { child, parent, parent_placement: placement, joint,
                child_actor: (child_handle, child_generation), parent_actor: (parent_handle, parent_generation),
                initial_render_frame, last_render_frame: None });
        }
        eprintln!("DOCUMENT_HIERARCHY phase=prepared bindings={} ancestors={} joints={}",
            self.document_hierarchy.bindings.len(), self.document_hierarchy.ancestors.len(),
            self.document_hierarchy.bindings.iter().filter(|binding| binding.joint >= 0).count());
        Ok(())
    }

    /// 8762d0/879100 remove the reciprocal hierarchy before identity release.
    /// The body remains in the same world at its current published transform.
    pub(super) fn detach_document(&mut self, live: &mut LiveEntities, child: usize) -> Result<(), String> {
        let Some(index) = self.document_hierarchy.bindings.iter().position(|binding| binding.child == child) else { return Ok(()); };
        let binding = &self.document_hierarchy.bindings[index];
        let child_actor = self.attachment_actor(child, binding.child_actor)?;
        let parent_actor = self.attachment_actor(binding.parent, binding.parent_actor)?;
        let position = self.actors.actors[parent_actor].children.iter().position(|&value| value == child_actor)
            .ok_or("bound child is absent from actor parent")?;
        live.unlink_child(self.entities[binding.parent].shared, self.entities[child].shared)?;
        self.actors.actors[parent_actor].children.remove(position);
        let actor = &mut self.actors.actors[child_actor];
        actor.parent = None;
        actor.binding.entity = None;
        self.entities[child].entity_flags &= !0x80;
        let parent_index=binding.parent;
        let joint=binding.joint;
        self.document_hierarchy.bindings.remove(index);
        if !self.document_hierarchy.bindings.iter().any(|binding|binding.parent==parent_index&&binding.joint==joint) {
            self.document_hierarchy.external_joints.remove(&(parent_index,joint));
        }
        self.actors.propagate_body_flag(parent_actor, false)
    }

    fn attachment_actor(&self, entity: usize, (handle, generation): (ComponentHandle, u32)) -> Result<usize, String> {
        let actor = self.actors.actors.get(handle.slot).ok_or("attachment actor slot retired")?;
        if actor.owner.generation != generation { return Err("attachment actor generation is stale".into()); }
        let linked = self.entities[entity].components.reference_actor == Some(handle);
        let retained = self.retirement.as_ref().is_some_and(|retirement| retirement.retained_actors.contains(&(handle, generation)));
        let bound = self.document_hierarchy.bindings.iter().any(|binding|
            (binding.child == entity && binding.child_actor == (handle, generation))
                || (binding.parent == entity && binding.parent_actor == (handle, generation)));
        if !linked && !retained && !bound { return Err("attachment actor has no live or retiring owner".into()); }
        Ok(handle.slot as usize)
    }

    /// Whole-world shutdown keeps attachment topology during component Stop.
    /// The actual entity destruction boundary owns reciprocal removal.
    pub(super) fn destroy_document_attachments(&mut self, live: &mut LiveEntities, entity: usize) -> Result<(), String> {
        while let Some(child) = self.document_hierarchy.bindings.iter().rev().find(|binding| binding.parent == entity).map(|binding| binding.child) {
            self.detach_document(live, child)?;
        }
        self.detach_document(live, entity)
    }

    pub(super) fn finish_document_hierarchy_retirement(&mut self) -> Result<(), String> {
        if !self.document_hierarchy.bindings.is_empty() { return Err("document attachments survived entity destruction".into()); }
        self.document_hierarchy.ancestors.clear();
        self.document_hierarchy.render_frames.clear();
        self.document_hierarchy.external_joints.clear();
        eprintln!("DOCUMENT_HIERARCHY phase=retired bindings=0 ancestors=0 frames=0");
        Ok(())
    }

    /// 808880 has a separate render virtual: it composes the parent's live
    /// render frame and phase3 joint, without moving the physics body. The
    /// existing final mesh/shadow/query publication owner consumes the delta.
    fn publish_bound_renders(&mut self, live: &LiveEntities,
        frames: &super::super::joint_frames::JointFrames,
        bound: &super::super::model_motion::BoundModels,
        placements: &super::super::RenderSourcePlacements,
        pending: &mut super::super::model_motion::ModelPoseUpdates) -> Result<(), String> {
        use crate::gameplay::reference_frame::bind;
        for (&index, &(initial, placement)) in &self.document_hierarchy.ancestors {
            // 808880 prefers the parent's actual current render frame. Only
            // a genuinely absent render entity uses the entity frame path.
            let current = if let Some(placement) = placement {
                let model = placements.0.get(placement).ok_or("bound ancestor render placement retired")?;
                if model.id != self.entities[index].name { return Err("bound ancestor placement identity differs".into()); }
                ReferenceFrame { position: model.position, basis: model.rotation }
            } else if let Some(delta) = bound.entity_delta(&self.entities[index].name) {
                let basis: [[f32; 3]; 3] = std::array::from_fn(|axis| std::array::from_fn(|row| delta[row][axis]));
                compose(ReferenceFrame { position: [delta[0][3], delta[1][3], delta[2][3]], basis }, initial)
            } else { self.entities[index].frame.pose };
            *self.document_hierarchy.render_frames.get_mut(&index).ok_or("bound ancestor render frame absent")? = current;
        }
        for binding in &mut self.document_hierarchy.bindings {
            // Ancestor render owners keep publishing their own motion. Only
            // the actually admitted RigidBody invokes808880 here; base entity
            // ancestors supply dependency frames, never a replacement renderer.
            let Some(&placement) = self.rigid_placements.get(&binding.child) else { continue; };
            let child = &self.entities[binding.child];
            if live.get(child.shared).is_none() || live.parent(child.shared)? != Some(self.entities[binding.parent].shared) {
                return Err("bound render attachment identity is stale".into());
            }
            let actor = child.components.reference_actor.and_then(|handle| self.actors.actors.get(handle.slot))
                .ok_or("bound render actor absent")?;
            let parent = *self.document_hierarchy.render_frames.get(&binding.parent).ok_or("bound parent render frame absent")?;
            let current = if binding.joint < 0 { bind(parent, actor.binding.offset, actor.binding.orientation) } else {
                let local=if let Some(frame)=self.document_hierarchy.external_joints.get(&(binding.parent,binding.joint)) {
                    bind(*frame,actor.binding.offset,actor.binding.orientation)
                } else {
                    let placement=binding.parent_placement.ok_or("bound joint render placement absent")?;
                    frames.local_frame(placement,binding.joint,3,actor.binding.offset,actor.binding.orientation)?
                        .ok_or("live bound joint frame absent")?
                };
                compose(parent,local)
            };
            *self.document_hierarchy.render_frames.get_mut(&binding.child).ok_or("bound child render frame absent")? = current;
            // 878c60/878d10 make808880 return immediately for hidden/retired
            // render entities. Otherwise publish only actual changed inputs.
            if child.entity_flags & 3 != 0 || binding.last_render_frame.is_some_and(|last| last.position == current.position && last.basis == current.basis) { continue; }
            let delta = super::rigid_actor::render_delta(binding.initial_render_frame, current)?;
            pending.0.try_reserve(1).map_err(|error| format!("bound render publication: {error}"))?;
            pending.0.push(super::super::model_motion::ModelPose { placement, delta });
            binding.last_render_frame = Some(current);
        }
        Ok(())
    }
}

pub(in crate::app) fn update_bound_renders(
    scene: Option<bevy::prelude::ResMut<PhysicsScene>>,
    live: Option<bevy::prelude::Res<LiveEntities>>,
    frames: Option<bevy::prelude::Res<super::super::joint_frames::JointFrames>>,
    bound: Option<bevy::prelude::Res<super::super::model_motion::BoundModels>>,
    placements: Option<bevy::prelude::Res<super::super::RenderSourcePlacements>>,
    pending: Option<bevy::prelude::ResMut<super::super::model_motion::ModelPoseUpdates>>,
) {
    let Some(mut scene) = scene else { return; };
    if scene.retirement.is_some() || scene.document_hierarchy.bindings.is_empty() { return; }
    let result = (|| {
        let mut pending = pending.ok_or("bound render publisher absent")?;
        scene.publish_bound_renders(live.as_deref().ok_or("bound live entity owner absent")?,
            frames.as_deref().ok_or("bound joint frame owner absent")?,
            bound.as_deref().ok_or("bound model motion owner absent")?,
            placements.as_deref().ok_or("bound render placement owner absent")?, &mut pending)
    })();
    if let Err(error) = result { bevy::log::error!("Bound readable render publication failed: {error}"); }
}

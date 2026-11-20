//! Placed arkCpntContactDamage sources and the player's touch consumer.
use super::PhysicsScene;
use crate::{
    archive::Catalog,
    assets::{damage::Declaration, declaration_texts, entities::{self, components, Value}},
    gameplay::{collision_user_data::CollisionUserData, component_state::lifecycle::ComponentLifecycleWorld,
        damage::Parameters, game_clock_runtime::ClockRuntime, player::Player},
};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Source {
    damage: Arc<Declaration>,
    amount: i32,
    interval: f32,
    walk_filter: bool,
}

impl PhysicsScene {
    pub(super) fn prepare_contact_damage(&mut self, catalog: &Catalog, document: &entities::Document) -> Result<(), String> {
        let declarations = document.definitions.iter().filter(|decl| decl.kind == "component")
            .map(|decl| (decl.id.as_str(), decl)).collect::<HashMap<_, _>>();
        let entities = document.definitions.iter().filter(|decl| decl.kind == "entity")
            .map(|decl| (decl.id.as_str(), decl)).collect::<HashMap<_, _>>();
        let mut component_fields = HashMap::new();
        let mut damage_declarations = HashMap::<String, Arc<Declaration>>::new();
        let mut read = |path: &str| declaration_texts::read_common_or_catalog(catalog, path);
        for (index, actor) in self.entities.iter().enumerate() {
            let Some(decl) = entities.get(actor.name.as_str()) else { continue };
            let Some(entries) = entities::scope(entities::field(&decl.fields, "edit"))
                .and_then(|fields| entities::scope(entities::field(fields, "m_componentDecls"))) else { continue };
            for (_, value) in entries.iter().filter(|(key, _)| key.starts_with("item_add[")) {
                let Value::Scope(entry) = value else { continue };
                let Some(fields) = components::entry_fields(entry, "cpntcontactdamage", &declarations, &mut component_fields, &mut read)? else { continue };
                let scheduling = components::scheduling(&fields,
                    crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0))?;
                if !scheduling.starts_on_spawn() { continue; }
                let Some(name) = crate::gameplay::damage::atom(&fields, "m_damageDecl")
                    .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("null")) else { continue };
                let damage = if let Some(damage) = damage_declarations.get(name) { damage.clone() } else {
                    let damage = Declaration::acquire(catalog, name)?;
                    damage_declarations.insert(name.to_owned(), damage.clone());
                    damage
                };
                // Defaults from 14070d870. 14070dcc0 enables the physics
                // component's touch bit when the damage component starts.
                let amount = crate::gameplay::damage::atom(&fields, "m_damageAmount").unwrap_or("1")
                    .parse().map_err(|_| format!("{}: invalid contact damage amount", actor.name))?;
                let interval = crate::gameplay::damage::number(&fields, "m_damageTimer", 1.)?;
                let walk_filter = match crate::gameplay::damage::atom(&fields, "m_walkFilter") {
                    None | Some("true" | "1") => true,
                    Some("false" | "0") => false,
                    Some(value) => return Err(format!("{}: invalid contact walk filter {value}", actor.name)),
                };
                self.contact_damage.insert(index, Source { damage, amount, interval, walk_filter });
                break;
            }
        }
        Ok(())
    }

    pub(in crate::app) fn update_player_contact_damage(&self, player: &mut Player, clock: &ClockRuntime) -> Result<(), String> {
        if self.contact_damage.is_empty() { return Ok(()); }
        let Some(target) = player.managed_entity() else { return Ok(()) };
        let origin = player.physics.position();
        let axis = player.physics.axis();
        let step = player.physics.posture_settings(player.physics.posture).step_height;
        let active = &mut player.contact_damage;
        active.current.clear();
        // 1403cf240 -> 14070dc10: consume the character's actual body
        // contacts and limit walking-only sources to the posture step height.
        for contact in player.physics.proxy.contact_hits().filter(|hit| hit.user_data != 0) {
            let data = CollisionUserData::decode(contact.user_data)?;
            let Some(index) = self.owner(data.component).ok().and_then(|owner| owner.entity) else { continue };
            let Some(source) = self.contact_damage.get(&index) else { continue };
            let entity = &self.entities[index];
            if entity.retired || entity.entity_flags & 1 != 0 || source.walk_filter && contact.position.z - origin.z > step { continue; }
            let entry = (index, entity.shared);
            if !active.current.contains(&entry) { active.current.push(entry); }
        }
        active.touching.retain(|entry| active.current.contains(entry));
        for &entry in &active.current {
            if !active.touching.contains(&entry) { active.touching.push(entry); }
        }
        let group = |index: usize| Arc::as_ptr(&self.contact_damage[&index].damage) as usize;
        active.last_hit.retain(|&(key, _)| active.touching.iter().any(|&(index, _)| group(index) == key));

        // Keep the player owner available to the existing health callback,
        // reusing our contact storage on the next accepted frame.
        let mut active = std::mem::take(active);
        let result = (|| {
            for (order, &(index, source_id)) in active.touching.iter().enumerate().rev() {
                let key = group(index);
                if active.touching[order + 1..].iter().any(|&(later, _)| group(later) == key) { continue; }
                let source = &self.contact_damage[&index];
                let now = clock.entity_time_from_definition(source_id, self.entities[index].timespace)?;
                if let Some((_, last)) = active.last_hit.iter_mut().find(|(group, _)| *group == key) {
                    if ((now - *last) as f32) <= source.interval { continue; }
                    *last = now;
                } else {
                    active.last_hit.push((key, now));
                }
                let mut params = Parameters::from_declaration(&source.damage);
                params.set_amount(&source.damage, source.amount as f32, true);
                params.hit_location = Some(origin);
                params.hit_normal = Some(axis.z_axis);
                params.hit_direction = -axis.x_axis;
                params.attacker = Some(target);
                params.inflictor = Some(source_id);
                player.receive_contact_damage(&params, &source.damage);
            }
            Ok(())
        })();
        player.contact_damage = active;
        result
    }
}

//! Pickable component state owned by the level component factory.
use super::component_state::{ComponentInstance, ComponentOwner};

/// One decoded component declaration shared by the components that actually
/// reference it. Inline declarations have their own owner.
#[derive(Debug)]
pub struct PickableDefinition {
    /// Same Pickable_Simple component; holding never creates inventory data.
    pub(crate) hold: Option<HoldProperties>,
    pub item: String,
    pub(crate) charm: Option<crate::assets::bone_charms::Placement>,
    pub quantity: i32,
    pub(crate) begin_pickup_sound: Option<std::sync::Arc<crate::assets::wwise::SoundEventDeclaration>>,
    pub shadow_grapple: bool,
    pub usable_by_player: bool,
    pub invisible_npc_loot: bool,
    pub(crate) pickup_interaction: super::interaction_definition::InteractionDef,
    /// Authored localization keys retained once with the shared declaration.
    pub(crate) interaction_labels: [String; 2],
    pub(crate) interaction_state: String,
    pub scheduling: super::component_state::declaration::DeclarationScheduling,
}

/// arkDeclCpntPickable_Simple::holdAndThrowProps_t, ctor 14076e0e0.
#[derive(Debug)]
pub(crate) struct HoldProperties {
    pub center_offset: [f32; 3],
    pub angle_offset: [[f32; 3]; 3],
    pub encumbrance: i32,
    pub both_hands: bool,
    pub attach_tag: String,
    pub collide_with_sword: bool,
    pub throw_factor: f32,
}

/// A checked component reference carries the factory generation, not just a
/// reusable pool slot. Each entry can be absent after the original lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckedComponent {
    pub handle: super::component_state::entity::ComponentHandle,
    pub generation: u32,
}

#[derive(Default)]
pub struct PickableReferences {
    pub player_audio: Option<CheckedComponent>,
    pub physics_base: Option<CheckedComponent>,
    pub physics_system: Option<CheckedComponent>,
    pub outsider_mark: Option<CheckedComponent>,
}

/// Inventory item data is acquired by the shared source at admission.
pub struct PickableComponent {
    /// The actual reserved entry for this generation of a derived Pickable.
    pub(crate) charm: Option<usize>,
    pub owner: ComponentOwner,
    pub definition: std::sync::Arc<PickableDefinition>,
    pub interaction_enabled: bool,
    /// 76e750 copies the declaration record into this retained payload.
    pub(crate) interaction: super::interaction_definition::InteractionDef,
    /// Low four native bits at +d0, initialized by 76e030.
    pub(crate) interaction_flags: u8,
    /// The component and interaction owner share one immutable declaration.
    pub item: Option<std::sync::Arc<crate::assets::inventory_item::PickableItem>>,
    pub references: Option<PickableReferences>,
    ///76e030's +28 starts at managed-null0x3fff0010;76e2b0 saves a
    ///previous real binding parent here. Managed-null is NOT all-ones(-1).
    pub(crate) pickup_parent:Option<crate::gameplay::entity_lifetime::EntityId>,
    /// Outer None means the game-local player lookup has not been supplied.
    pub player: Option<Option<crate::gameplay::entity_lifetime::EntityId>>,
    /// 76ee70 updates this checked pointer when a loot owner associates the
    /// pickup; the 76e030 constructor initializes it empty.
    pub loot_owner: Option<CheckedComponent>,
}

impl PickableComponent {
    /// 76e030 -> 77c270: the base interactable starts enabled and the
    /// pickable requests updates through the common component owner.
    pub fn new(definition: std::sync::Arc<PickableDefinition>) -> Self {
        let mut owner = ComponentOwner::default();
        owner.request_update(1);
        // 76e030 constructs this payload independently; 76e750 later copies
        // the declaration UID along with its other fields.
        let interaction = super::interaction_definition::InteractionDef::new();
        Self { owner, definition, charm: None, interaction_enabled: true, interaction, interaction_flags: 0,
            item: None, references: None, pickup_parent:None, player: None, loot_owner: None }
    }

    /// 76e750's virtual record callback. The host supplies the live
    /// 877530/6dd500 owner classification, rather than guessing from a
    /// placement name or treating an unavailable NPC owner as an empty result.
    pub(crate) fn interaction_records(&mut self, owner_forbids: bool)
        -> Option<&super::interaction_definition::InteractionDef> {
        if self.interaction_flags & 2 != 0 || owner_forbids { return None; }
        self.interaction = self.definition.pickup_interaction;
        if let Some(item) = &self.item {
            if let Some(crate::assets::inventory_item::PickableReadable::Note(note)
                | crate::assets::inventory_item::PickableReadable::Other(note, _)) = &item.readable {
                let name = &note.title;
                self.interaction.name = if name.is_empty() { 0 } else { crate::assets::lang::key_id(name) };
            }
        }
        Some(&self.interaction)
    }

    /// 76e860: initialize the common owner and publish the entity flags.
    pub fn initialize(&mut self, entity_flags: &mut u32, entity_flags_d7: &mut u8) {
        self.owner.mark_initialized();
        *entity_flags_d7 |= 4;
        *entity_flags = (*entity_flags & !(1 << 15)) | (u32::from(self.definition.shadow_grapple) << 15);
    }
}

impl ComponentInstance for PickableComponent {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}

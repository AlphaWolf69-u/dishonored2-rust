//! AISmartObject5681b0/5b2f40, Investigate984de0 and SearchDynamic5af570.
//! The shared Midnight owner retains scene identities; NPC playback requests
//! its own projection when it actually uses a slot.
use crate::gameplay::options::decl::scope as child;
use crate::{
    archive::Catalog,
    entities::Value,
    gameplay::options::decl::{atom, boolean, float, int},
};
use std::sync::Arc;

pub struct Slot {
    pub offset: [f32; 3],
    pub orientation: [[f32; 3]; 3],
    pub go_around_radius: f32,
    pub position_tolerance: f32,
    pub vertical_tolerance: f32,
    pub maximum_users: i32,
    pub goto_use_location: bool,
    pub run_to_location: bool,
    pub follow_location: bool,
    pub fallback: bool,
    pub require_face_to: bool,
    /// Authored enum spelling is retained until a user requests orientation.
    pub face_to_mode: Option<Box<str>>,
    pub face_to_entity: Option<Box<str>>,
    pub look_at_entity: bool,
    pub blocking: bool,
    pub equipment_state: Option<Box<str>>,
    pub require_equipment_state: bool,
    pub maximum_suspicion: Option<Box<str>>,
    pub snap_duration: f32,
    pub tag: Option<Box<str>>,
    pub(crate) scene: Option<Arc<crate::assets::midnight_scene::Definition>>,
    pub loop_count: i32,
    pub retry_priority: Option<Box<str>>,
}
pub struct Common {
    pub slots: Vec<Slot>,
    pub selection_order: Option<Box<str>>,
    pub check_navmesh: bool,
    pub offset_in_entity_space: bool,
    pub detach_when_finished: bool,
    pub stop_midnight_on_stop: bool,
}
pub enum Definition {
    Investigate {
        common: Common,
        aggregation_range: f32,
        track_allowed: bool,
    },
    SearchDynamic {
        common: Common,
        influence: f32,
        influence_to_seed: f32,
        seed_tolerance: f32,
        required_tag: Option<Box<str>>,
    },
}
fn text(fields: &[(String, Value)], name: &str) -> Option<Box<str>> {
    atom(fields, name).filter(|v| !v.is_empty()).map(Into::into)
}
fn vector(fields: &[(String, Value)], name: &str, default: [f32; 3]) -> [f32; 3] {
    let Some(fields) = child(fields, name) else {
        return default;
    };
    [
        float(fields, "x", default[0]),
        float(fields, "y", default[1]),
        float(fields, "z", default[2]),
    ]
}
fn orientation(fields: &[(String, Value)]) -> [[f32; 3]; 3] {
    let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let orientation =
        if let Some(fields) = child(fields, "m_orientation").and_then(|f| child(f, "mat")) {
            [
                vector(fields, "mat[0]", identity[0]),
                vector(fields, "mat[1]", identity[1]),
                vector(fields, "mat[2]", identity[2]),
            ]
        } else {
            identity
        };
    orientation
}
fn slot(catalog: &Catalog, fields: &[(String, Value)]) -> Result<Slot, String> {
    let orientation = orientation(fields);
    let scene = atom(fields, "m_sceneDecl")
        .filter(|s| !s.is_empty())
        .map(|name| {
            crate::assets::midnight_scene::acquire_reference(catalog, name)
                .map_err(|e| e.to_string())
        })
        .transpose()?;
    let face = child(fields, "m_faceToEntity");
    Ok(Slot {
        offset: vector(fields, "m_positionOffset", [0.; 3]),
        orientation,
        go_around_radius: float(fields, "m_goAroundRadius", 0.),
        position_tolerance: float(fields, "m_positionTolerance", 0.),
        vertical_tolerance: float(fields, "m_verticalTolerance", 0.1),
        maximum_users: int(fields, "m_maxUsers", 1),
        goto_use_location: boolean(fields, "m_gotoUseLoc", true),
        run_to_location: boolean(fields, "m_runToUseLoc", false),
        follow_location: boolean(fields, "m_followUseLoc", false),
        fallback: boolean(fields, "m_fallbackSlot", false),
        require_face_to: boolean(fields, "m_requireFaceTo", true),
        face_to_mode: text(fields, "m_faceToMode"),
        face_to_entity: face
            .and_then(|f| text(f, "m_entity"))
            .or_else(|| text(fields, "m_faceToEntity")),
        look_at_entity: face.map_or(true, |f| boolean(f, "m_lookAtEntity", true)),
        blocking: boolean(fields, "m_blockingSlot", true),
        equipment_state: text(fields, "m_equipState"),
        require_equipment_state: boolean(fields, "m_equipStateRequired", true),
        maximum_suspicion: text(fields, "m_maxSuscpicionStateAllowed"),
        snap_duration: float(fields, "m_snapOnStartDuration", 0.3),
        tag: text(fields, "m_tag"),
        scene,
        loop_count: int(fields, "m_loopNumber", 1),
        retry_priority: text(fields, "m_midnightRetryMaxPriority"),
    })
}
impl Definition {
    pub fn read(
        catalog: &Catalog,
        kind: &str,
        fields: &[(String, Value)],
        inline: bool,
    ) -> Result<Self, String> {
        let mut slots = Vec::new();
        if let Some(values) = child(fields, "m_slots") {
            let count = int(values, "num", 0);
            if count < 0 {
                return Err("AISmartObject slot count is negative".into());
            }

            for index in 0..count as usize {
                slots.push(slot(
                    catalog,
                    child(values, &format!("item[{index}]")).unwrap_or(&[]),
                )?);
            }
        }
        //5b5b80 checks actual declaration+5c bit0, m_isInlineDecl. A file
        //declaration with no slots must not receive the deprecated slot.
        if slots.is_empty() && boolean(fields, "m_isInlineDecl", inline) {
            let mut value = slot(catalog, &[])?;
            value.offset = vector(fields, "m_useOffset", [0.; 3]);
            value.orientation = orientation(fields);
            value.position_tolerance = float(fields, "m_positionTolerance", 0.1);
            value.goto_use_location = boolean(fields, "m_gotoUseLoc", true);
            value.require_face_to = boolean(fields, "m_requireFaceTo", true);
            value.scene = atom(fields, "m_sceneDecl")
                .filter(|s| !s.is_empty())
                .map(|name| {
                    crate::assets::midnight_scene::acquire_reference(catalog, name)
                        .map_err(|e| e.to_string())
                })
                .transpose()?;
            value.loop_count = int(fields, "m_loopNumber", 1);
            slots.push(value);
        }
        let common = Common {
            slots,
            selection_order: text(fields, "m_selectionOrder"),
            check_navmesh: boolean(fields, "m_checkNavmeshAtRuntime", false),
            offset_in_entity_space: boolean(fields, "m_offsetInEntitySpace", true),
            detach_when_finished: boolean(fields, "m_detachWhenUseFinished", false),
            stop_midnight_on_stop: boolean(fields, "m_stopMidnightOnStopUsed", true),
        };
        match kind {
            "cpntaiinvestigatespot" => Ok(Self::Investigate {
                common,
                aggregation_range: float(fields, "m_searchProxyAggregationRange", -1.),
                track_allowed: boolean(fields, "m_trackAllowed", true),
            }),
            "cpntaisearchspotdynamic" => Ok(Self::SearchDynamic {
                common,
                influence: float(fields, "m_searchInfluence", 0.1),
                influence_to_seed: float(fields, "m_influenceToSeed", 0.),
                seed_tolerance: float(fields, "m_seedTolerance", 2.),
                required_tag: text(fields, "m_tagRequiredForActivation"),
            }),
            _ => Err(format!("unknown AISmartObject subtype {kind}")),
        }
    }
    pub fn common(&self) -> &Common {
        match self {
            Self::Investigate { common, .. } | Self::SearchDynamic { common, .. } => common,
        }
    }
    pub fn factory_name(&self) -> &'static [u8] {
        match self {
            Self::Investigate { .. } => b"arkCpntAIInvestigateSpot",
            Self::SearchDynamic { .. } => b"arkCpntAISearchSpot_Dynamic",
        }
    }
}

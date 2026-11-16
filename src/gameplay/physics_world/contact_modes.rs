//! ce4cc0,cf1c30/cf1d10/cf1f00/cf1fe0 trigger response policy.
pub(crate) mod validation;

use super::contacts::ContactCallbackState;
use crate::gameplay::materials::MaterialRecord;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Response {
    #[default]
    Contact,
    Broadphase,
    Narrowphase,
    ContactSolver,
}
impl Response {
    pub(crate) fn from_materials(materials: [&MaterialRecord; 2]) -> Result<Self, String> {
        let kinds = materials.map(|m| (m.words[2] & 255) as u8);
        if kinds.iter().any(|&kind| kind > 3) {
            return Err("physics material response mode invalid".into());
        }
        Ok(if kinds.contains(&1) {
            Self::Broadphase
        } else if kinds.contains(&3) {
            Self::ContactSolver
        } else if kinds.contains(&2) {
            Self::Narrowphase
        } else {
            Self::Contact
        })
    }
    pub(crate) fn from_leaf_materials(materials: [&MaterialRecord; 2]) -> Result<Self, String> {
        let kinds = materials.map(|m| (m.words[2] & 255) as u8);
        if kinds.iter().any(|&kind| kind > 3) {
            return Err("physics material response mode invalid".into());
        }
        Ok(if kinds.contains(&3) {
            Self::ContactSolver
        } else if kinds.iter().any(|&kind| kind != 0) {
            Self::Narrowphase
        } else {
            Self::Contact
        })
    }
    pub(crate) fn initialize(self, notify: bool, cache: &mut ContactCallbackState) -> Option<u16> {
        if self == Self::Contact {
            return None;
        }
        cache.cache_flags |= 4; //cache+0a modifier callback policy, not +22 event bits.
        if !notify || self != Self::ContactSolver {
            cache.modifier_flags |= 0x200; //source trigger-stream response.
            if notify {
                cache.contact_flags |= 8;
                return Some(1);
            }
        }
        None
    }
    pub(crate) fn collide(
        self,
        notify: bool,
        separation: f32,
        cache: &mut ContactCallbackState,
    ) -> Option<u16> {
        if self == Self::Contact || !notify {
            return None;
        }
        if 0. <= separation {
            cache.contact_flags &= !0x10;
            None
        } else {
            cache.contact_flags |= 0x10;
            if cache.contact_flags & 8 == 0 {
                cache.contact_flags |= 8;
                Some(1)
            } else {
                None
            }
        }
    }
    pub(crate) fn retire(self, cache: &mut ContactCallbackState) -> Option<u16> {
        if self == Self::Contact {
            return None;
        }
        let event = (cache.contact_flags & 8 != 0).then_some(0x101);
        cache.contact_flags &= !0x18;
        event
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriggerEvent {
    pub bodies: [u32; 2],
    pub shape_keys: [u32; 2],
    pub flags: u16,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriggerTransition {
    pub trigger: u32,
    pub other: u32,
    pub userdata: [u64; 2],
    pub other_filter_info: u32,
    pub entered: bool,
}

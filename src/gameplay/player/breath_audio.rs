//! Breath sound-event A9 and all-object RTPC AA producers4c0b50/4c0c60/4c1160.
use crate::{
    archive::Catalog,
    assets::{
        entities::{scene_motion::atom, Value},
        wwise::{RtpcDeclaration, SoundEventDeclaration},
    },
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Runtime {
    pub start: Option<Arc<SoundEventDeclaration>>,
    pub stop: Option<Arc<SoundEventDeclaration>>,
    pub rtpc: Option<Arc<RtpcDeclaration>>,
    started: bool,
    value: Option<f32>,
    ///Host admission lease; media/decoders remain in the shared audio owner.
    pub host_scope: Option<Arc<str>>,
    pub host_ready: bool,
}
impl Runtime {
    pub fn acquire(catalog: &Catalog, fields: &[(String, Value)]) -> Result<Option<Self>, String> {
        let path = |name| {
            atom(fields, name)
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
        };
        let event = |name| {
            path(name)
                .map(|path| {
                    crate::assets::wwise::event_resources::acquire_with(catalog, path, |path| {
                        crate::assets::declaration_texts::read_common_or_catalog(catalog, path)
                    })
                })
                .transpose()
        };
        let start = event("m_startBreathingSoundEvent")?;
        let stop = event("m_stopBreathingSoundEvent")?;
        let rtpc = path("m_breathingSoundRTPC")
            .map(|path| {
                crate::assets::wwise::rtpc_resources::acquire_with(catalog, path, |path| {
                    crate::assets::declaration_texts::read_common_or_catalog(catalog, path)
                })
            })
            .transpose()?;
        if start.is_none() && stop.is_none() && rtpc.is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            start,
            stop,
            rtpc,
            started: false,
            value: None,
            host_scope: None,
            host_ready: false,
        }))
    }
    ///4c0cf0 suppresses this heavy-breathing loop in modes1/2/3. Underwater
    ///sound is a separate Swimming FX producer; never force this loop there.
    pub fn update(&mut self, breath: f32, suppressed: bool) -> Option<u32> {
        if suppressed {
            if self.started {
                self.value = Some(0.);
                return self.stop();
            }
            return None;
        }
        if breath >= 100. {
            return self.stop();
        }
        let post = if !self.started && self.start.is_some() && self.stop.is_some() {
            self.started = true;
            self.start.as_ref().map(|event| event.id)
        } else {
            None
        };
        if self.started {
            self.value = Some(1. - breath * 0.01);
        }
        post
    }
    pub fn stop(&mut self) -> Option<u32> {
        if !self.started {
            return None;
        }
        self.started = false;
        self.stop.as_ref().map(|event| event.id)
    }
    pub fn parameter(&self) -> Option<(u32, f32)> {
        self.rtpc
            .as_ref()
            .zip(self.value)
            .map(|(rtpc, value)| (rtpc.id, value))
    }
}

//! Host adapter for the canonical statistics arrays and the rewrite profile
//! dictionary. The resource persists across map resources; save I/O remains
//! owned by the existing profile/save system.
use bevy::prelude::*;
use crate::gameplay::{entity_events::{EntityEvents, Event, Payload}, entity_lifetime::EntityId,
    player_statistics::{self as statistics, Profile, Scope, Statistic}};
use std::collections::BTreeMap;

/// host: replaces the platform statistics +e0/+60 interface with an actual
/// session profile dictionary. Increment and set are both received as in
/// 67aea0/67e330; the final value is the manager's cumulative signed count.
struct SessionProfile { dictionary: BTreeMap<String, String> }
impl Profile for SessionProfile {
    fn available(&self) -> bool { true }
    fn read(&self, statistic: Statistic) -> Option<i32> {
        self.dictionary.get(statistic.profile_key()).map(|value| value.trim().parse().unwrap_or(0))
    }
    fn increment(&mut self, statistic: Statistic, quantity: i32) {
        let previous = self.read(statistic).unwrap_or(0);
        self.write(statistic, previous.wrapping_add(quantity));
    }
    fn write(&mut self, statistic: Statistic, value: i32) {
        self.dictionary.insert(statistic.profile_key().to_owned(), value.to_string());
    }
}

#[derive(Resource)]
pub(super) struct PlayerStatistics {
    owner: statistics::PlayerStatistics,
    profile: SessionProfile,
}
impl Default for PlayerStatistics {
    fn default() -> Self { Self::from_profile(BTreeMap::new()) }
}
impl PlayerStatistics {
    pub fn from_profile(dictionary: BTreeMap<String, String>) -> Self {
        let profile = SessionProfile { dictionary };
        let mut owner = statistics::PlayerStatistics::default();
        owner.load_profile(&profile);
        Self { owner, profile }
    }

    /// Call once during initial preparation. Missing profile keys use the
    /// native zero default; this reads the rewrite's existing profile owner.
    pub fn prepare() -> Self {
        let folder = crate::platform::home::home().saves_dir();
        let dictionary = crate::gameplay::save_list::read_profile(&folder).unwrap_or_default();
        Self::from_profile(dictionary)
    }
    pub fn bind_chapter(&mut self, chapter: Option<i32>) { self.owner.bind_chapter(chapter); }
    pub fn increment(&mut self, id: u32) -> Result<bool, String> {
        let statistic = Statistic::from_id(id)?;
        Ok(self.owner.increment(statistic, &mut self.profile))
    }
    pub fn count(&self, statistic: Statistic, scope: Scope) -> i32 { self.owner.count(statistic, scope) }
    pub fn reset_chapter_statistic(&mut self, statistic: Statistic) { self.owner.reset_chapter_statistic(statistic, &mut self.profile); }
    pub fn saved_counts(&self) -> statistics::SavedCounts { self.owner.saved_counts() }
    pub fn restore(&mut self, saved: &statistics::SavedCounts, map_transition: bool) { self.owner.restore(saved, map_transition, &self.profile); }
    /// Give the actual profile save owner its dictionary; no alternate disk
    /// file, original-game save modification, or automatic save is invented.
    pub fn profile_dictionary(&self) -> &BTreeMap<String, String> { &self.profile.dictionary }

    /// 67aea0 emits StatisticUpdated1ca after the profile set. Root's event
    /// owner supplies the actual player EntityId (native null instigator).
    pub fn publish_updates(&mut self, player: EntityId, events: &mut EntityEvents) {
        for statistic in self.owner.take_updates() {
            events.post(Event { id: 0x1ca, target: Some(player), payload: Payload::Bytes(statistic.id().to_le_bytes().to_vec()) });
        }
    }
}

#[derive(Resource)]
pub(super) struct PreparedStatisticsChapter { chapter: Option<i32>, applied: bool }
impl PreparedStatisticsChapter {
    /// Same actual map-info +20c field read by 67bec0. Map matching remains
    /// in the existing map-info declaration owner, including chaos aliases.
    pub fn prepare(catalog: &crate::assets::archive::Catalog, map: &str) -> Self {
        let chapter = crate::gameplay::map_info::find(catalog, map)
            .and_then(|fields| crate::gameplay::map_info::loading_info(&fields)).map(|info| info.mission_index);
        Self { chapter, applied: false }
    }
    pub fn prepare_current(catalog: &crate::assets::archive::Catalog) -> Self {
        Self { chapter: crate::gameplay::map_info::current_loading_info(catalog).map(|info| info.mission_index), applied: false }
    }
}

/// Publish each level's chapter once while preserving the session/profile
/// arrays across ordinary map retirement. Journal mission changes may call
/// bind_chapter directly when its actual chapter resolver dispatches67d240.
pub(super) fn bind_prepared_chapter(mut statistics: ResMut<PlayerStatistics>, prepared: Option<ResMut<PreparedStatisticsChapter>>) {
    let Some(mut prepared) = prepared else { return };
    if prepared.applied { return; }
    statistics.bind_chapter(prepared.chapter);
    prepared.applied = true;
}

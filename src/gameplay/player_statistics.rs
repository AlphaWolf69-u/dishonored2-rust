//! Ordinary potion statistics in arkPlayerStatisticsManager. The manager
//! owns chapter/playthrough/profile arrays; the platform adapter owns its
//! profile dictionary. Counts are signed 32-bit values, not telemetry.
//! Evidence: 67ac70, 67aea0, 67bce0, 67bc00, 67c730, 67cc50,
//! 67d1e0/67d240, 67d3f0, 67d850, 67dc10 and 67e330.
pub const COUNT: usize = 46;
pub type Counts = [i32; COUNT];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Statistic { HealthPotion = 0x16, ManaPotion = 0x17 }
impl Statistic {
    pub fn from_id(id: u32) -> Result<Self, String> {
        match id { 0x16 => Ok(Self::HealthPotion), 0x17 => Ok(Self::ManaPotion),
            _ => Err(format!("ordinary potion statistics owner does not handle statistic {id:#x}")) }
    }
    pub fn id(self) -> u32 { self as u32 }
    pub fn profile_key(self) -> &'static str {
        match self {
            Self::HealthPotion => "ARK_PLAYER_STATISTIC_USE_HEALTH_POTION",
            Self::ManaPotion => "ARK_PLAYER_STATISTIC_USE_MANA_POTION",
        }
    }
}

/// 67c730's exact scope numbers: chapter1, current playthrough2, profile3.
#[derive(Clone, Copy)]
pub enum Scope { Chapter = 1, Playthrough = 2, Profile = 3 }

/// Native statSet_t is one chapter ID followed by 46 signed counts (0xbc).
#[derive(Clone)]
pub struct StatSet { pub chapter: i32, pub counts: Counts }
impl StatSet { fn empty(chapter: i32) -> Self { Self { chapter, counts: [0; COUNT] } } }

/// The understood ordinary-counter projection of 67dc10's save block.
/// Profile totals are deliberately excluded: 67cc50 reloads them from the
/// actual profile dictionary after the save block has been restored.
#[derive(Clone)]
pub struct SavedCounts { pub chapters: Vec<StatSet>, pub playthrough: Counts }

/// Native +e0 supplies an interface, its +60 receives the increment, and the
/// profile dictionary is read/written through 657ce0/67d3f0/67e330. A host
/// backend implements this contract; absence suppresses the native mutation.
pub trait Profile {
    fn available(&self) -> bool;
    fn read(&self, statistic: Statistic) -> Option<i32>;
    fn increment(&mut self, statistic: Statistic, quantity: i32);
    fn write(&mut self, statistic: Statistic, value: i32);
}

pub struct PlayerStatistics {
    chapters: Vec<StatSet>,
    playthrough: Counts,
    cumulative: Counts,
    chapter: Option<i32>,
    updates: Vec<Statistic>,
}
impl Default for PlayerStatistics {
    fn default() -> Self {
        // 67ac70 zeroes the two arrays and admits the pending chapter -1.
        Self { chapters: vec![StatSet::empty(-1)], playthrough: [0; COUNT],
            cumulative: [0; COUNT], chapter: None, updates: Vec::new() }
    }
}
impl PlayerStatistics {
    fn chapter_index(&mut self, chapter: i32) -> usize {
        if let Some(index) = self.chapters.iter().position(|set| set.chapter == chapter) { return index; }
        self.chapters.push(StatSet::empty(chapter));
        self.chapters.len() - 1
    }

    /// 67bec0 receives the chapter from the live Journal or actual map-info
    /// loadingInfo.m_missionIndex (+20c). Unknown chapters do not count.
    /// 67d240 transfers any pending -1 record into the resolved chapter.
    pub fn bind_chapter(&mut self, chapter: Option<i32>) {
        self.chapter = chapter.filter(|id| *id >= 0);
        if let Some(chapter) = self.chapter {
            let index = self.chapter_index(chapter);
            let pending = self.chapter_index(-1);
            self.merge_chapter(pending, index);
        }
    }

    fn merge_chapter(&mut self, source: usize, target: usize) {
        if source == target { return; }
        let counts = self.chapters[source].counts;
        for (current, value) in self.chapters[target].counts.iter_mut().zip(counts) { *current = current.wrapping_add(value); }
        self.chapters[source].counts.fill(0);
    }

    pub fn load_profile(&mut self, profile: &impl Profile) {
        if !profile.available() { return; }
        for statistic in [Statistic::HealthPotion, Statistic::ManaPotion] {
            self.cumulative[statistic as usize] = profile.read(statistic).unwrap_or(0);
        }
    }

    /// 67aea0 gates on a live platform interface AND a nonnegative chapter,
    /// then increments all three arrays and broadcasts StatisticUpdated1ca.
    pub fn add(&mut self, statistic: Statistic, quantity: i32, profile: &mut impl Profile) -> bool {
        if !profile.available() { return false; }
        let Some(chapter) = self.chapter else { return false };
        let index = self.chapter_index(chapter);
        let counter = statistic as usize;
        self.chapters[index].counts[counter] = self.chapters[index].counts[counter].wrapping_add(quantity);
        self.playthrough[counter] = self.playthrough[counter].wrapping_add(quantity);
        self.cumulative[counter] = self.cumulative[counter].wrapping_add(quantity);
        profile.increment(statistic, quantity);
        profile.write(statistic, self.cumulative[counter]);
        self.updates.push(statistic);
        true
    }

    pub fn increment(&mut self, statistic: Statistic, profile: &mut impl Profile) -> bool { self.add(statistic, 1, profile) }

    pub fn count(&self, statistic: Statistic, scope: Scope) -> i32 {
        let index = statistic as usize;
        match scope {
            Scope::Chapter => self.chapter.and_then(|chapter| self.chapters.iter().find(|set| set.chapter == chapter)).map_or(0, |set| set.counts[index]),
            Scope::Playthrough => self.playthrough[index], Scope::Profile => self.cumulative[index],
        }
    }

    /// 67bc00 resets only this chapter's contribution, subtracting it from
    /// playthrough and profile arrays, and writing the profile key. It does
    /// not replay a negative platform increment or broadcast1ca.
    pub fn reset_chapter_statistic(&mut self, statistic: Statistic, profile: &mut impl Profile) {
        let Some(chapter) = self.chapter else { return };
        let index = self.chapter_index(chapter);
        let counter = statistic as usize;
        let removed = std::mem::replace(&mut self.chapters[index].counts[counter], 0);
        self.playthrough[counter] = self.playthrough[counter].wrapping_sub(removed);
        self.cumulative[counter] = self.cumulative[counter].wrapping_sub(removed);
        if profile.available() { profile.write(statistic, self.cumulative[counter]); }
    }

    pub fn saved_counts(&self) -> SavedCounts { SavedCounts { chapters: self.chapters.clone(), playthrough: self.playthrough } }

    /// 67d850 replaces on ordinary restore and adds on transition restore.
    /// The caller must deliver a saved block once, then retire it (67cc50).
    pub fn restore(&mut self, saved: &SavedCounts, transition: bool, profile: &impl Profile) {
        if !transition { self.chapters.clear(); self.playthrough.fill(0); }
        for set in &saved.chapters {
            let index = self.chapter_index(set.chapter);
            for (current, value) in self.chapters[index].counts.iter_mut().zip(set.counts) { *current = current.wrapping_add(value); }
        }
        for (current, value) in self.playthrough.iter_mut().zip(saved.playthrough) { *current = current.wrapping_add(value); }
        self.load_profile(profile);
        self.updates.clear();
    }

    pub fn take_updates(&mut self) -> impl Iterator<Item = Statistic> + '_ { self.updates.drain(..) }
}

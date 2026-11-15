//! The physics world's source-owned timespace/profile mapping (140092c30).
//! Derived records live in the one motion-properties library, not this owner.
use super::{motion_properties::MotionPropertiesLibrary, signal::Signal};

struct Space { factor: f32, mapping: Vec<u16>, derived: Vec<bool> }
struct Pending { base: u16, target: usize, next: usize, values: Vec<u16> }
struct Step {
    deltas: [f32; 2], maximum: f32, factors: [f32; 2], changed: [bool; 2],
    any_old_zero: bool, profile: usize, space: usize, profiles_done: bool, bodies_done: bool,
}

pub(crate) struct MotionTimespaces { spaces: Vec<Space>, pending: Option<Pending>, preset_next: u8, step: Option<Step> }
impl MotionTimespaces {
    /// 1400932d0 sizes the four arrays from its supplied timespace count and
    /// initializes each current/previous factor to 1.0.
    pub(crate) fn new(count: usize) -> Result<Self, String> {
        let mut spaces = Vec::new();
        spaces.try_reserve_exact(count).map_err(|error| format!("physics timespaces: {error}"))?;
        spaces.resize_with(count, || Space { factor: 1.0, mapping: Vec::new(), derived: Vec::new() });
        Ok(Self { spaces, pending: None, preset_next: 0, step: None })
    }

    /// 1400932d0 resolves the five constructor profiles in ascending order.
    /// Retain the index so allocation failure cannot duplicate earlier IDs.
    pub(crate) fn initialize_presets<S: Copy + Eq, C: Copy + Eq>(
        &mut self, library: &mut MotionPropertiesLibrary<S, C>,
        mut dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16),
    ) -> Result<(), String> {
        while self.preset_next < 5 {
            self.resolve(library, u16::from(self.preset_next), 0, &mut dispatch)?;
            self.preset_next += 1;
        }
        Ok(())
    }
    pub(crate) fn presets_ready(&self) -> bool { self.preset_next == 5 }
    pub(crate) fn factor(&self, space: usize) -> Result<f32, String> {
        self.spaces.get(space).map(|value| value.factor).ok_or_else(|| "physics timespace is absent".into())
    }

    /// 1400944f0 is the game world's motion-property-removed listener. It
    /// removes both directions of each pair and clears both derived bits.
    pub(crate) fn forget_profile(&mut self, id: u16) -> Result<(), String> {
        if self.pending.is_some() || self.step.is_some() {
            return Err("motion profile removal overlaps an unfinished timespace operation".into());
        }
        for space in &self.spaces {
            let index = usize::from(id);
            if index >= space.mapping.len() { continue; }
            let other = usize::from(space.mapping[index]);
            if other >= space.mapping.len() || other >= space.derived.len() || index >= space.derived.len() {
                return Err("motion profile removal has no reciprocal mapping".into());
            }
        }
        for space in &mut self.spaces {
            let index = usize::from(id);
            if index >= space.mapping.len() { continue; }
            let other = usize::from(space.mapping[index]);
            space.mapping[index] = u16::MAX;
            space.mapping[other] = u16::MAX;
            space.derived[index] = false;
            space.derived[other] = false;
        }
        Ok(())
    }

    /// 140097bc0 selects the largest positive group delta and compares each
    /// normalized factor with the live factor using its 2^-23 threshold.
    /// No positive delta means the native world does not advance.
    pub(crate) fn begin_step(&mut self, deltas: [f32; 2]) -> Result<Option<f32>, String> {
        if self.spaces.len() != 2 || !self.presets_ready() || self.pending.is_some() || self.step.is_some() {
            return Err("physics timespace owner is not ready for a step".into());
        }
        let mut maximum = None;
        for delta in deltas {
            if delta > 0.0 && maximum.is_none_or(|old| delta > old) { maximum = Some(delta); }
        }
        let Some(maximum) = maximum else { return Ok(None) };
        let factors = deltas.map(|delta| delta / maximum);
        let mut changed = [false; 2];
        let mut any_old_zero = false;
        for index in 0..2 {
            let old = self.spaces[index].factor;
            let difference = f32::from_bits((old - factors[index]).to_bits() & 0x7fff_ffff);
            changed[index] = difference >= f32::from_bits(0x3400_0000);
            any_old_zero |= changed[index] && old == 0.0;
        }
        if !changed.into_iter().any(|value| value) { return Ok(Some(maximum)); }
        self.step = Some(Step { deltas, maximum, factors, changed, any_old_zero,
            profile: 0, space: 0, profiles_done: false, bodies_done: false });
        Ok(Some(maximum))
    }
    pub(crate) fn step_requires_update(&self) -> bool { self.step.is_some() }

    /// Updates the shared library in native base-ID/space order before body
    /// velocity changes. The cursor survives a failed slot or callback owner.
    pub(crate) fn update_step_profiles<S: Copy + Eq, C: Copy + Eq>(&mut self,
        library: &mut MotionPropertiesLibrary<S, C>,
        mut dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16)) -> Result<(), String> {
        let step = self.step.as_mut().ok_or("physics timespace step is absent")?;
        while step.profile < library.slot_count() {
            let base = *library.get(u16::try_from(step.profile).map_err(|_| "motion profile index overflow")?)
                .ok_or("motion profile slot is absent")?;
            if base.words()[14] & 0xff == 0xff || base.words()[1] & 0x0400_0000 != 0 {
                step.profile += 1;
                step.space = 0;
                continue;
            }
            while step.space < self.spaces.len() {
                let space = &self.spaces[step.space];
                if step.changed[step.space] {
                    let id = *space.mapping.get(step.profile).ok_or("motion profile has no timespace mapping")?;
                    if id == u16::MAX { return Err("motion profile timespace mapping is invalid".into()); }
                    library.replace(id, base.with_timespace_scale(step.factors[step.space]), &mut dispatch)?;
                }
                step.space += 1;
            }
            step.profile += 1;
            step.space = 0;
        }
        step.profiles_done = true;
        Ok(())
    }

    /// The caller uses the all-body branch when a changed old factor was
    /// zero; otherwise it visits the actual active-body list in its order.
    pub(crate) fn step_body_policy(&self) -> Result<(bool, [f32; 2], [f32; 2]), String> {
        let step = self.step.as_ref().ok_or("physics timespace step is absent")?;
        if !step.profiles_done { return Err("physics timespace profiles are not updated".into()); }
        Ok((step.any_old_zero, [self.spaces[0].factor, self.spaces[1].factor], step.factors))
    }

    pub(crate) fn finish_body_pass(&mut self) -> Result<(), String> {
        let step = self.step.as_mut().ok_or("physics timespace step is absent")?;
        if !step.profiles_done { return Err("physics timespace profiles are not updated".into()); }
        step.bodies_done = true;
        Ok(())
    }

    /// Called only after the real ordered velocity pass succeeds. Physics
    /// simulation then consumes the returned largest advancing delta.
    pub(crate) fn finish_step(&mut self, deltas: [f32; 2]) -> Result<f32, String> {
        let step = self.step.as_ref().ok_or("physics timespace step is absent")?;
        if !step.profiles_done || !step.bodies_done || step.deltas.map(f32::to_bits) != deltas.map(f32::to_bits) {
            return Err("physics timespace step inputs changed or velocity pass is incomplete".into());
        }
        let maximum = step.maximum;
        for (space, factor) in self.spaces.iter_mut().zip(step.factors) { space.factor = factor; }
        self.step = None;
        Ok(maximum)
    }

    /// The 140851a70 listener checks this bit before translating the active
    /// motion-property ID back to the original shared profile.
    pub(crate) fn original_for_derived(&self, space: usize, current: u16) -> Result<Option<u16>, String> {
        let space = self.spaces.get(space).ok_or("physics timespace is absent")?;
        let index = usize::from(current);
        if !space.derived.get(index).copied().unwrap_or(false) { return Ok(None); }
        let original = *space.mapping.get(index).ok_or("derived motion mapping is absent")?;
        if original == u16::MAX { return Err("derived motion mapping is invalid".into()); }
        Ok(Some(original))
    }

    /// 140094280 scans spaces in source order and uses the first whose
    /// derived-profile bit contains this live motion property.
    pub(crate) fn factor_for_derived(&self, current: u16) -> Option<f32> {
        self.spaces.iter().find_map(|space| {
            space.derived.get(usize::from(current)).copied().unwrap_or(false).then_some(space.factor)
        })
    }

    /// 140092c30 finds the selected-space reverse mapping first. If absent,
    /// it creates a profile in every space, then publishes both directions and
    /// marks each derived ID. A pending request keeps successfully allocated
    /// library IDs across failure rather than allocating duplicates on retry.
    pub(crate) fn resolve<S: Copy + Eq, C: Copy + Eq>(&mut self, library: &mut MotionPropertiesLibrary<S, C>,
        base: u16, target: usize, mut dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16)) -> Result<u16, String> {
        if self.step.is_some() { return Err("physics timespace step is incomplete".into()); }
        let space = self.spaces.get(target).ok_or("target physics timespace is absent")?;
        if let Some(pending) = &self.pending {
            if pending.base != base || pending.target != target { return Err("another motion timespace mapping is incomplete".into()); }
        } else if let Some(index) = space.mapping.iter().position(|&entry| entry == base) {
            return u16::try_from(index).map_err(|_| "motion timespace reverse index overflow".into());
        } else {
            library.get(base).ok_or("base motion property is absent")?;
            let mut values = Vec::new();
            values.try_reserve_exact(self.spaces.len()).map_err(|error| format!("motion timespace profiles: {error}"))?;
            self.pending = Some(Pending { base, target, next: 0, values });
        }
        let pending = self.pending.as_mut().ok_or("motion timespace request is absent")?;
        while pending.next < self.spaces.len() {
            let mut profile = (*library.get(base).ok_or("base motion property is absent")?)
                .with_timespace_scale(self.spaces[pending.next].factor);
            profile.set_sharing(1);
            let id = library.add(profile, &mut dispatch)?;
            pending.values.push(id);
            pending.next += 1;
        }
        let max_id = pending.values.iter().copied().fold(base, u16::max);
        let length = self.spaces.iter().map(|space| space.mapping.len()).max().unwrap_or(0)
            .max(usize::from(max_id) + 1);
        for space in &mut self.spaces {
            if space.mapping.len() < length {
                space.mapping.try_reserve(length - space.mapping.len()).map_err(|error| format!("motion timespace map: {error}"))?;
                space.derived.try_reserve(length - space.derived.len()).map_err(|error| format!("motion timespace flags: {error}"))?;
            }
        }
        for (space, &id) in self.spaces.iter_mut().zip(&pending.values) {
            space.mapping.resize(length, u16::MAX);
            space.derived.resize(length, false);
            space.mapping[usize::from(base)] = id;
            space.mapping[usize::from(id)] = base;
            space.derived[usize::from(id)] = true;
        }
        let result = pending.values[target];
        self.pending = None;
        Ok(result)
    }
}

//! FX sound objects, their playing IDs, and inaudible-event replay (22f7a0..22fec0).
use bevy_math::Vec3;

pub struct SoundHandle<O, P> {
    object: Option<O>,
    events: Vec<u32>,
    playing: Vec<P>,
    queued: Vec<u32>,
    queued_range: f32,
    allow_virtualization: bool,
    virtualize: bool,
    persistent_loop: bool,
}
impl<O, P> Default for SoundHandle<O, P> {
    fn default() -> Self {
        Self {
            object: None,
            events: Vec::new(),
            playing: Vec::new(),
            queued: Vec::new(),
            queued_range: 0.,
            allow_virtualization: false,
            virtualize: false,
            persistent_loop: false,
        }
    }
}

/// Required live audio operations. Posting receives the handle because the
/// game's start/end callbacks can run before PostEvent returns its playing ID.
pub trait SoundObjectWorld {
    type Object: Copy;
    type Playing: Copy + PartialEq;
    type Context;
    fn available(&self) -> bool;
    /// Allocate the ordinary FX object and apply its initial position/axes.
    fn allocate(
        &mut self,
        position: Vec3,
        context: &Self::Context,
    ) -> Result<Option<Self::Object>, String>;
    fn post(
        &mut self,
        handle: &mut SoundHandle<Self::Object, Self::Playing>,
        object: Self::Object,
        event: u32,
        context: &Self::Context,
    ) -> Result<Option<Self::Playing>, String>;
    fn release(&mut self, object: Self::Object, stop: bool) -> Result<(), String>;
    /// 22fac0 writes the physics-energy RTPC only after successful first post.
    fn initialize_posted(
        &mut self,
        object: Self::Object,
        context: &Self::Context,
    ) -> Result<(), String>;
    fn object_inaudible(&mut self, object: Self::Object) -> Result<bool, String>;
    fn object_range(&self, object: Self::Object) -> Result<f32, String>;
    /// The ordinary c1240 point query, using the retained object's range.
    fn point_inaudible(&mut self, position: Vec3, range: f32) -> Result<bool, String>;
    /// 22fe30 publishes position/axes and the actual context time-scale RTPC.
    fn update_position(
        &mut self,
        object: Self::Object,
        position: Vec3,
        context: &Self::Context,
    ) -> Result<(), String>;
}

impl<O: Copy, P: Copy + PartialEq> SoundHandle<O, P> {
    pub fn object(&self) -> Option<O> {
        self.object
    }

    /// 22f8f0 counts events and queued events, not object allocation.
    pub fn is_alive(&self) -> bool {
        !self.events.is_empty() || !self.queued.is_empty()
    }

    /// 22f960 changes only flag0; current playback/callback state is retained.
    pub fn allow_virtualization(&mut self, enabled: bool) {
        self.allow_virtualization = enabled;
    }

    /// c4550's callback cookie stores max distance, the requested callback
    /// mask, and b3850's persistent-loop flag. Callback8/8000 ORs that flag.
    pub fn event_info(&mut self, range: f32, requested_callbacks: u32, persistent_loop: bool) {
        self.persistent_loop |= persistent_loop;
        self.virtualize = self.allow_virtualization
            && range > 0.
            && self.persistent_loop
            && requested_callbacks & 4 == 0;
    }

    pub fn event_info_for(&mut self, playing: P, range: f32, callbacks: u32, persistent_loop: bool) {
        if self.playing.contains(&playing) { self.event_info(range, callbacks, persistent_loop); }
    }

    /// Callback1/22f970 removes the first matching ID and its parallel event
    /// reference with swap removal, preserving the source's later replay order.
    pub fn finished(&mut self, playing: P) {
        if let Some(index) = self.playing.iter().position(|&id| id == playing) {
            self.events.swap_remove(index);
            self.playing.swap_remove(index);
        }
    }

    /// 22f870 clears only queued references and flags0/1. Active voices and
    /// callback flag2 remain until their own completion or full handle reset.
    pub fn clear_queued(&mut self) {
        self.queued.clear();
        self.queued_range = 0.;
        self.allow_virtualization = false;
        self.virtualize = false;
    }

    pub fn post<W: SoundObjectWorld<Object = O, Playing = P>>(
        &mut self,
        world: &mut W,
        event: u32,
        position: Vec3,
        context: &W::Context,
    ) -> Result<bool, String> {
        if let Some(object) = self.object {
            return self.post_existing(world, object, event, context);
        }
        if !world.available() {
            return Ok(false);
        }
        let object = world
            .allocate(position, context)?
            .ok_or("FX sound object pool is exhausted")?;
        self.object = Some(object);
        if self.post_existing(world, object, event, context)? {
            world.initialize_posted(object, context)?;
            Ok(true)
        } else {
            world.release(object, true)?;
            *self = Self::default();
            Ok(false)
        }
    }

    fn post_existing<W: SoundObjectWorld<Object = O, Playing = P>>(
        &mut self,
        world: &mut W,
        object: O,
        event: u32,
        context: &W::Context,
    ) -> Result<bool, String> {
        let Some(id) = world.post(self, object, event, context)? else {
            return Ok(false);
        };
        self.events.push(event);
        self.playing.push(id);
        Ok(true)
    }

    /// 22fc30/22fd00: evict an inaudible replayable object, retain its event
    /// references, then restore them last-first when its source becomes audible.
    pub fn update<W: SoundObjectWorld<Object = O, Playing = P>>(
        &mut self,
        world: &mut W,
        position: Vec3,
        context: &W::Context,
    ) -> Result<(), String> {
        if let Some(object) = self.object {
            if self.virtualize && world.object_inaudible(object)? {
                std::mem::swap(&mut self.events, &mut self.queued);
                self.playing.clear();
                self.queued_range = world.object_range(object)?;
                world.release(object, true)?;
                self.object = None;
                return Ok(());
            }
        } else if let Some(&event) = self.queued.last() {
            if !world.point_inaudible(position, self.queued_range)?
                && self.post(world, event, position, context)?
            {
                self.queued.pop();
                while let Some(event) = self.queued.pop() {
                    let object = self
                        .object
                        .ok_or("FX sound replay lost its allocated object")?;
                    self.post_existing(world, object, event, context)?;
                }
            }
        }
        if let Some(object) = self.object {
            world.update_position(object, position, context)?;
        }
        Ok(())
    }

    /// 22f920 resets the handle only when it owns an allocated object.
    pub fn release<W: SoundObjectWorld<Object = O, Playing = P>>(
        &mut self,
        world: &mut W,
    ) -> Result<(), String> {
        if let Some(object) = self.object {
            world.release(object, true)?;
            *self = Self::default();
        }
        Ok(())
    }
}

//! Component declaration scheduling fields (00cc90/00cd20/74f770).
#[derive(Clone, Debug)]
pub struct DeclarationScheduling {
    pub update_mask: u32,
    pub start_phases: u32,
    pub priority: i32,
    pub flags: u8,
}
impl DeclarationScheduling {
    /// Reflected98ff10/990570 property, independent of the update-mask setter.
    pub fn starts_on_spawn(&self) -> bool {
        self.flags & 4 != 0
    }
    pub fn set_start_on_spawn(&mut self, enabled: bool) {
        self.flags = (self.flags & !4) | (u8::from(enabled) << 2);
    }
    /// 00cc90 initializes these fields after the generic resource/declaration
    /// constructor. Unrelated upper flag bits survive construction.
    pub fn from_base_flags(flags: u8) -> Self {
        Self {
            update_mask: 0,
            start_phases: 0x1f,
            priority: 8,
            flags: (flags & 0xfc) | 4,
        }
    }
    /// 74f770 adds the reference-frame component's scheduling mask.
    pub fn reference_frame(flags: u8) -> Self {
        let mut state = Self::from_base_flags(flags);
        state.add_updates(0x10);
        state
    }
    /// Complete00cd20: OR update phases and derive flag4 from mask bits0/2.
    pub fn add_updates(&mut self, mask: u32) {
        self.flags &= !4;
        self.update_mask |= mask;
        if self.update_mask & 5 == 0 {
            self.flags |= 4;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(value: &DeclarationScheduling) -> [u32; 4] {
        [
            value.update_mask,
            value.start_phases,
            value.priority as u32,
            u32::from(value.flags),
        ]
    }
    #[test]
    fn spawn_property_matches_executed_reflected_accessors() {
        for row in include_str!("../../../tests/fixtures/component_start_on_spawn.txt").lines() {
            let words: Vec<_> = row.split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
            let mut declaration = DeclarationScheduling::from_base_flags(0);
            declaration.flags = words[0] as u8;
            declaration.set_start_on_spawn(words[1] != 0);
            assert_eq!(u32::from(declaration.flags), words[2]);
            assert_eq!(u32::from(declaration.starts_on_spawn()), words[3]);
        }
    }
    #[test]
    fn scheduling_matches_executed_common_and_reference_declaration_constructors() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/component_declaration.txt").lines() {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let flags = words.next().unwrap() as u8;
            let reference = words.next().unwrap() != 0;
            let mut state = if reference {
                DeclarationScheduling::reference_frame(flags)
            } else {
                DeclarationScheduling::from_base_flags(flags)
            };
            assert_eq!(
                snapshot(&state),
                std::array::from_fn(|_| words.next().unwrap())
            );
            for _ in 0..3 {
                state.add_updates(words.next().unwrap());
                assert_eq!(
                    snapshot(&state),
                    std::array::from_fn(|_| words.next().unwrap())
                );
            }
            assert!(words.next().is_none());
            cases += 1;
        }
        assert_eq!(cases, 512);
    }
}

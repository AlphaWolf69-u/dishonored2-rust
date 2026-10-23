//! Position-only boat programs, admitted by their actual cached stage pair.
use super::{ProgramLibrary, Result};
use crate::render_program::{
    RenderParameterKind, SerializedParameter, SerializedParameterBlock, SerializedParameterValue,
};

impl ProgramLibrary {
    /// Model passes force arkBoatVolume; world passes retain the material MAIN.
    /// Return its definition state after proving the selected position-only ABI.
    pub fn boat_volume_program_state(
        &mut self,
        name: &[u8],
        options: &SerializedParameterBlock,
        world: bool,
    ) -> Result<[u32; 9]> {
        let mut options = options.clone();
        options.parameters.push(SerializedParameter {
            name: b"useMultiInstancing".to_vec(),
            kind: RenderParameterKind::Option,
            value: SerializedParameterValue::Words([world as u32, 0, 0, 0]),
        });
        let variant = self
            .serialized_variant(name, &options)?
            .ok_or("boat volume program needs runtime compilation")?;
        let pair = self.stage_checksums(&variant)?;
        let expected = if world {
            [
                0x65, 0x82, 0x5b, 0x7d, 0xdd, 0x6c, 0x6e, 0xc0, 0xba, 0x7a, 0x54, 0x34, 0x47, 0x18,
                0xc7, 0xce,
            ]
        } else {
            [
                0x78, 0x6c, 0x59, 0xbe, 0xb2, 0xf0, 0x23, 0x7b, 0x2f, 0xf5, 0x1a, 0x92, 0x02, 0xe6,
                0xb1, 0x29,
            ]
        };
        // The legacy world variant writes only SV_Target0, with no discard,
        // SV_Depth or UAV side effects. 293400 disables that target's colour
        // mask. Its clip output matches VS148 for all 512 executed cases, so
        // its unused lighting varyings/pixel stage can be omitted in this pass.
        let legacy_world = world
            && pair
                == [
                    Some([
                        0xd4, 0x1a, 0xb7, 0x36, 0xf4, 0xe4, 0x21, 0x86, 0x79, 0xfc, 0x07, 0x76,
                        0xab, 0x1e, 0xfb, 0x19,
                    ]),
                    Some([
                        0x62, 0x6f, 0x81, 0x9e, 0x5d, 0x56, 0x1d, 0x85, 0xe1, 0x55, 0x9a, 0x36,
                        0xc5, 0x20, 0x51, 0x30,
                    ]),
                ];
        if !legacy_world && pair != [Some(expected), None] {
            return Err(format!("unsupported boat volume stages {pair:02x?}"));
        }
        Ok(self
            .definition(name)?
            .ok_or("boat volume program definition unavailable")?
            .render_state())
    }
}

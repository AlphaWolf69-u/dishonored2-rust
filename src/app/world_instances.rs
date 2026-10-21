//! One immutable, game-coordinate matrix stream for every world-geometry pass.
use bevy::{prelude::*, render::storage::ShaderBuffer};

/// Level-owned, independently of whether sunlight or local shadows are enabled.
/// Replacing the level must replace this owner together with its scene data.
#[derive(Resource, Default)]
pub(super) struct WorldMatrices(Option<Handle<ShaderBuffer>>);

impl WorldMatrices {
    pub(super) fn get_or_upload(
        &mut self,
        matrices: &[[f32; 12]],
        buffers: &mut Assets<ShaderBuffer>,
    ) -> Option<Handle<ShaderBuffer>> {
        if self.0.is_none() && !matrices.is_empty() {
            // The world shaders index original 48-byte affine rows. No host
            // coordinate conversion, decomposition, or per-pass copy belongs here.
            self.0 = Some(buffers.add(ShaderBuffer::from(matrices.to_vec())));
        }
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumers_share_exact_level_rows_without_reuploading() {
        let matrices = [
            [
                1.0, -2.0, 3.0, 4.0, 5.0, 6.0, -7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
            ],
            [
                -0.0, 0.25, 0.5, 16.0, -0.125, 2.0, 0.0, -32.0, 0.0, 0.0, 4.0, 64.0,
            ],
        ];
        let mut owner = WorldMatrices::default();
        let mut buffers = Assets::<ShaderBuffer>::default();
        assert!(owner.get_or_upload(&[], &mut buffers).is_none());
        assert!(buffers.is_empty());
        let first = owner.get_or_upload(&matrices, &mut buffers).unwrap();
        let expected: Vec<_> = matrices
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(
            buffers.get(&first).unwrap().data.as_ref().unwrap(),
            &expected
        );
        assert_eq!(expected.len(), 2 * 48);
        // Extraction consumes CPU bytes. Later consumers must retain the actual
        // GPU allocation's identity without treating missing CPU data as a reload.
        buffers.get_mut(&first).unwrap().data = None;
        for _ in 0..4 {
            assert_eq!(
                owner.get_or_upload(&matrices, &mut buffers).unwrap().id(),
                first.id()
            );
        }
        assert_eq!(buffers.len(), 1);
        assert!(buffers.get(&first).unwrap().data.is_none());
        let next_level = WorldMatrices::default()
            .get_or_upload(&matrices, &mut buffers)
            .unwrap();
        assert_ne!(next_level.id(), first.id());
    }
}

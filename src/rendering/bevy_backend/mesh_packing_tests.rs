//! The vendored allocator's vertex interleave against Bevy's own packing.
use bevy::mesh::{Mesh, MeshVertexAttribute, PrimitiveTopology, VertexFormat};

/// Fixed-size interleaving writes exactly `Mesh::write_packed_vertex_buffer_data`'s
/// bytes for 4-, 8-, 12- and 16-byte attributes in the mesh's order.
#[test]
fn packed_vertex_data_matches_bevy() {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    let n = 37;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, (0..n).map(|i| [i as f32, 1.5 * i as f32, -(i as f32)]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, (0..n).map(|i| [0.25 * i as f32, 2.0, 3.0]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, (0..n).map(|i| [i as f32 * 0.5, 7.0]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, (0..n).map(|i| [1.0, i as f32, 3.0, -1.0]).collect::<Vec<_>>());
    const SCALAR: MeshVertexAttribute = MeshVertexAttribute::new("DH2_TEST_SCALAR", 0x5eed_0001, VertexFormat::Float32);
    mesh.insert_attribute(SCALAR, (0..n).map(|i| i as f32 * 3.0).collect::<Vec<_>>());
    let mut expected = vec![0u8; mesh.get_vertex_buffer_size()];
    mesh.write_packed_vertex_buffer_data(wgpu::WriteOnly::from_mut(&mut expected[..]));
    assert_eq!(bevy::render::mesh::allocator::packed_vertex_data(&mesh), expected);
}

use super::*;
use std::collections::HashMap;

#[test]
fn world_water_retains_every_snorm_corner_without_changing_forward_normals() {
    use bevy::{
        asset::RenderAssetUsages,
        mesh::{Mesh, VertexAttributeValues},
        render::render_resource::PrimitiveTopology,
    };
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let positions = vec![[2.0, -3.0, 7.0]; 256];
    let normals = vec![[0.0, 1.0, 0.0]; 256];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone());
    let packed: Vec<[u8; 16]> = (0..=255u8)
        .map(|w| {
            let mut v = [0; 16];
            v[7] = w;
            v
        })
        .collect();
    retain_normal_w(&mut mesh, &packed).unwrap();
    let Some(VertexAttributeValues::Float32(actual)) = mesh.attribute(NORMAL_W) else {
        panic!("missing water corner channel")
    };
    for (value, byte) in actual.iter().zip(0..=255u8) {
        assert_eq!(
            value.to_bits(),
            ((byte as i8 as f32 / 127.0).max(-1.0)).to_bits()
        );
    }
    assert!(
        matches!(mesh.attribute(Mesh::ATTRIBUTE_POSITION), Some(VertexAttributeValues::Float32x3(v)) if v == &positions)
    );
    assert!(
        matches!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL), Some(VertexAttributeValues::Float32x3(v)) if v == &normals)
    );
    assert!(retain_normal_w(&mut mesh, &packed[..255]).is_err());
    assert!(retain_normal_w(&mut mesh, &[[0; 8]; 256]).is_err());
}

#[test]
#[ignore = "requires graphics adapter; compares cached VS14494 with real PNW buffers"]
fn world_water_colour_vertex_matches_cached_stage() {
    let mut data: HashMap<&str, Vec<Vec<u32>>> = HashMap::new();
    for line in include_str!("../../../../tests/fixtures/water_world_vertex.txt").lines() {
        let mut fields = line.split_whitespace();
        let label = fields.next().unwrap();
        if label == "META" {
            continue;
        }
        data.entry(label).or_default().push(
            fields
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect(),
        );
    }
    let bytes = |words: &[u32]| {
        words
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let mut pnw = vec![0u32; 8];
    for (i, label) in ["OFFSETS", "REFS", "TETRA", "TETRA_VERTEX"]
        .into_iter()
        .enumerate()
    {
        pnw[i] = pnw.len() as u32;
        pnw.extend(data[label].iter().flatten().copied());
    }
    let buffers = [
        bytes(&data["REMAP"][0]),
        bytes(&data["MATRIX"].concat()),
        bytes(&pnw),
        bytes(&data["INFO"].concat()),
        bytes(&data["CUBE_REMAP"][0]),
    ];
    assert_eq!(data["EXPECTED"].len(), 512);
    let words = |w: &[u32]| {
        w.iter()
            .map(|v| format!("{v:08x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    // A nonzero allocator base verifies native-local vertex addressing.
    let mut header = data["VP"][0].clone();
    header.extend([3, 137, 2, 0]);
    let parameters = WaterWorldColourParameters {
        clip_from_world: std::array::from_fn(|r| {
            std::array::from_fn(|c| f32::from_bits(header[4 * r + c]))
        }),
        instance_buffer_offset: 3,
        first_vertex: 137,
        first_instance: 2,
    };
    assert_eq!(parameters.uniform_bytes().as_slice(), bytes(&header));
    let mut fixture = format!("frame {}\n", words(&header));
    for (i, expected) in data["EXPECTED"].iter().enumerate() {
        let mut query = data["VERTEX"][i % 16].clone();
        query.extend([137 + (i % 16) as u32, 2 + (i / 16) as u32, 0, 0]);
        fixture.push_str(&format!("{} {}\n", words(&query), words(expected)));
    }
    let source = format!("{SHADER_SOURCE}\n{}", include_str!("tests.wgsl"));
    let frames =
        super::super::test_water_inputs::compare_storage_inputs(&source, &fixture, 8, 12, &buffers);
    assert_eq!(frames, 1);
}

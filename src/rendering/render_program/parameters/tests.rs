use super::*;
use crate::render_program::{RenderParameterKind, SerializedParameterValue};
struct View([[u32; 4]; 4]);
impl ParameterSource for View {
    fn scalar(&self, _: &str) -> Result<f32, String> {
        Ok(f32::from_bits(self.0[1][0]))
    }
    fn vector(&self, _: &str) -> Result<[f32; 4], String> {
        Ok(self.0[0].map(f32::from_bits))
    }
    fn boolean(&self, _: &str) -> Result<bool, String> {
        Ok(self.0[2][0] != 0)
    }
    fn image_source(&self, _: &str) -> Result<String, String> {
        Ok(format!("image_{}", self.0[3][0]))
    }
}
#[test]
fn ordered_scopes_match_executed_publication() {
    let mut checked = 0;
    for line in include_str!("../../../../tests/fixtures/draw_parameter_scopes.txt").lines() {
        let data: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let view = View(std::array::from_fn(|i| {
            data[1 + i * 4..5 + i * 4].try_into().unwrap()
        }));
        let mut at = 17;
        let blocks: Vec<_> = (0..2)
            .map(|_| {
                let n = data[at] as usize;
                at += 1;
                let mut parameters = Vec::new();
                for _ in 0..n {
                    let id = data[at] as usize;
                    let words: [u32; 4] = data[at + 1..at + 5].try_into().unwrap();
                    at += 5;
                    parameters.push(SerializedParameter {
                        name: [b"VeCtOr\0ignored".as_slice(), b"SCALAR", b"flag", b"image"][id]
                            .to_vec(),
                        kind: if id == 3 {
                            RenderParameterKind::Texture
                        } else {
                            RenderParameterKind::Uniform
                        },
                        value: if id == 3 {
                            SerializedParameterValue::Name(
                                format!("image_{}\0ignored", words[0]).into_bytes(),
                            )
                        } else {
                            SerializedParameterValue::Words(words)
                        },
                    });
                }
                SerializedParameterBlock {
                    flags: 0,
                    parameters,
                }
            })
            .collect();
        let read = ParameterScopes {
            view: &view,
            blocks: &[&blocks[0], &blocks[1]],
        };
        assert_eq!(
            read.vector("vector").unwrap().map(f32::to_bits),
            data[at..at + 4]
        );
        assert_eq!(read.scalar("scalar").unwrap().to_bits(), data[at + 4]);
        assert_eq!(read.boolean("flag").unwrap(), data[at + 8] != 0);
        assert_eq!(
            read.image_source("image").unwrap(),
            format!("image_{}", data[at + 12])
        );
        assert_eq!(data.len(), at + 16);
        checked += 1;
    }
    assert_eq!(checked, 64);
}
#[test]
fn scopes_preserve_payload_types() {
    let view = View([[0; 4]; 4]);
    let block = SerializedParameterBlock {
        flags: 0,
        parameters: vec![SerializedParameter {
            name: b"value".to_vec(),
            kind: RenderParameterKind::Texture,
            value: SerializedParameterValue::Name(b"_white".to_vec()),
        }],
    };
    let read = ParameterScopes {
        view: &view,
        blocks: &[&block],
    };
    assert!(read.vector("value").is_err());
    assert!(read.scalar("value").is_err());
    assert!(read.boolean("value").is_err());
    assert_eq!(read.image_source("value").unwrap(), "_white");
}

//! Offline DHC1 replay. Uses the existing paired original capture/converter;
//! no archive conversion, graphics device, world launch or manufactured inputs.
use dishonored2_rust::tome::shadow_caster::*;
use std::{error::Error, path::Path, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at.checked_add(4).ok_or("offset overflow")?)
            .ok_or("truncated DHC1 word")?
            .try_into()?,
    ))
}
fn float(b: &[u8], at: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(b, at)?))
}
fn context(b: &[u8]) -> Result<([[f32; 3]; 2], Vec<[f32; 4]>, Vec<usize>)> {
    if b.len() != 0x590 {
        return Err("invalid plane context size".into());
    }
    let n = u32_at(b, 0x4ac)? as usize;
    if n > 22 {
        return Err("plane context exceeds original capacity".into());
    }
    let mut bounds = [[0.; 3]; 2];
    for side in 0..2 {
        for axis in 0..3 {
            bounds[side][axis] = float(b, 0x530 + side * 16 + axis * 4)?;
        }
    }
    let mut planes = Vec::with_capacity(n);
    let mut active = Vec::with_capacity(n);
    for i in 0..n {
        let mut p = [0.; 4];
        for a in 0..4 {
            p[a] = float(b, 0x80 + i * 16 + a * 4)?;
        }
        planes.push(p);
        active.push(u32_at(b, 0x4d4 + i * 4)? as usize);
    }
    Ok((bounds, planes, active))
}
pub fn run(args: &[String]) -> Result<()> {
    if args.len() < 2 {
        return Err("usage: tome-audit --gather-replay <capture-resource-root> <DHC1 capture>... [--repeats N]".into());
    }
    let root = Path::new(&args[0]);
    let mut paths = Vec::new();
    let mut repeats = 32usize;
    let mut at = 1;
    while at < args.len() {
        if args[at] == "--repeats" {
            at += 1;
            repeats = args.get(at).ok_or("missing repeats")?.parse()?;
        } else {
            paths.push(&args[at]);
        }
        at += 1;
    }
    if repeats == 0 || paths.is_empty() {
        return Err("nonzero repeats and paired captures required".into());
    }
    let mut baseline_cycles = 0u64;
    let mut candidate_cycles = 0u64;
    let mut all_cycles = true;
    for path in paths {
        let bytes = std::fs::read(path)?;
        if bytes.get(..4) != Some(b"DHC1") {
            return Err("expected existing paired DHC1 capture".into());
        }
        let path_len = u32_at(&bytes, 4)? as usize;
        let receiver_len = u32_at(&bytes, 8)? as usize;
        let descriptor_len = u32_at(&bytes, 12)? as usize;
        let capture_job = u32_at(&bytes, 16)?;
        let jobs = u32_at(&bytes, 20)?;
        let count = u32_at(&bytes, 24)? as usize;
        let output_count = u32_at(&bytes, 28)? as usize;
        if jobs != 8 || capture_job >= jobs {
            return Err("expected actual eight-job global capture".into());
        }
        if receiver_len != 0x86d0 + count.checked_mul(0x590).ok_or("receiver size overflow")? {
            return Err("invalid receiver size".into());
        }
        let path_end = 32usize.checked_add(path_len).ok_or("path size overflow")?;
        let source_path =
            std::str::from_utf8(bytes.get(32..path_end).ok_or("truncated Tome path")?)?;
        let source = std::fs::read(root.join(source_path))?;
        let tome = ShadowCasterTome::parse(&source)?;
        let receiver_end = path_end
            .checked_add(receiver_len)
            .ok_or("receiver offset overflow")?;
        let receiver = bytes
            .get(path_end..receiver_end)
            .ok_or("truncated receiver")?;
        if u32_at(receiver, 0x86bc)? & 16 != 0 {
            return Err("alternate mask mode not supported by existing global gather".into());
        }
        let descriptor_end = receiver_end
            .checked_add(descriptor_len)
            .ok_or("descriptor offset overflow")?;
        let descriptor = bytes
            .get(receiver_end..descriptor_end)
            .ok_or("truncated descriptor")?;
        let distance = match descriptor_len {
            0 => None,
            20 => Some(ShadowDistanceDescriptor {
                center: [
                    float(descriptor, 0)?,
                    float(descriptor, 4)?,
                    float(descriptor, 8)?,
                ],
                radius: float(descriptor, 12)?,
                override_center: descriptor[16] != 0,
            }),
            _ => return Err("invalid descriptor size".into()),
        };
        let (_, planes, active) = context(&receiver[0x8110..0x86a0])?;
        let primary = ShadowReceiverState::from_planes(&planes, &active);
        let contexts = (0..count)
            .map(|i| context(&receiver[0x86d0 + i * 0x590..0x86d0 + (i + 1) * 0x590]))
            .collect::<Result<Vec<_>>>()?;
        let cascades = contexts
            .iter()
            .map(|(bounds, planes, active)| ShadowReceiverState {
                bounds: *bounds,
                planes,
                active_planes: active,
            })
            .collect::<Vec<_>>();
        let mut rows = [[0.; 4]; 4];
        let mut transposed = [[0.; 4]; 4];
        let mut mask = Box::new([0; 16384]);
        for r in 0..4 {
            for c in 0..4 {
                rows[r][c] = float(receiver, r * 16 + c * 4)?;
                transposed[r][c] = float(receiver, 0x40 + r * 16 + c * 4)?;
            }
        }
        for (i, v) in mask.iter_mut().enumerate() {
            *v = u16::from_le_bytes(
                receiver
                    .get(0xf0 + i * 2..0xf2 + i * 2)
                    .ok_or("truncated mask")?
                    .try_into()?,
            );
        }
        let depth = (receiver[0x86b8] != 0).then_some(ReceiverDepthProjection {
            receiver_rows: rows,
            transposed_rows: transposed,
            mask,
        });
        let point = [
            float(receiver, 0x86a0)?,
            float(receiver, 0x86a4)?,
            float(receiver, 0x86a8)?,
        ];
        let direction = CasterDirection::from_receiver_direction([
            float(receiver, 0x86ac)?,
            float(receiver, 0x86b0)?,
            float(receiver, 0x86b4)?,
        ])?;
        if bytes.len()
            != descriptor_end + output_count.checked_mul(8).ok_or("output size overflow")?
        {
            return Err("invalid exact-output payload size".into());
        }
        let expected = (0..output_count)
            .map(|i| {
                Ok(GlobalShadowCasterHit {
                    tome_index: u32_at(&bytes, descriptor_end + i * 4)?,
                    cascade_mask: u32_at(&bytes, descriptor_end + output_count * 4 + i * 4)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut reference = GlobalShadowCasterScratch::new(tome.object_count(), count);
        let mut candidate = GlobalShadowCasterScratch::new(tome.object_count(), count);
        for job in 0..jobs {
            let input = GlobalShadowCasterInput {
                source: &source,
                primary,
                cascades: &cascades,
                distance: ShadowObjectDistanceQuery::from_descriptor(point, distance),
                direction,
                depth: depth.as_ref(),
                job,
                jobs,
            };
            let a = reference.gather_reference(&tome, &input)?;
            let b = candidate.gather(&tome, &input)?;
            if a != b {
                return Err(format!("ordered output mismatch {path} job{job}").into());
            }
            if job == capture_job && a != expected {
                return Err(format!("original oracle output mismatch {path} job{job}").into());
            }
            for block in 0..8 {
                for baseline in if block % 2 == 0 {
                    [true, false]
                } else {
                    [false, true]
                } {
                    let scratch = if baseline {
                        &mut reference
                    } else {
                        &mut candidate
                    };
                    let started = Instant::now();
                    let cycles = scratch.replay_cycles(&tome, &input, baseline, repeats)?;
                    let elapsed = started.elapsed();
                    if let Some(cycles) = cycles {
                        if baseline {
                            baseline_cycles += cycles;
                        } else {
                            candidate_cycles += cycles;
                        }
                    } else {
                        all_cycles = false;
                    }
                    println!(
                        "GATHER_AB capture={path:?} job={job} block={block} baseline={baseline} calls={repeats} cycles={cycles:?} elapsed_ms={:.6}",
                        elapsed.as_secs_f64() * 1000.
                    );
                }
            }
            if reference.gather_reference(&tome, &input)? != candidate.gather(&tome, &input)? {
                return Err("ordered output changed after replay".into());
            }
        }
        println!(
            "GATHER_ORACLE capture={path:?} original_job={capture_job} ordered_native_rows={output_count} all_jobs_equal=true"
        );
    }
    if all_cycles && baseline_cycles > 0 {
        let saving = (1. - candidate_cycles as f64 / baseline_cycles as f64) * 100.;
        println!(
            "GATHER_AB_TOTAL baseline_cycles={baseline_cycles} candidate_cycles={candidate_cycles} saving_percent={saving:.3} predicted_percent=7.5 investigate_prediction_mismatch={}",
            (saving - 7.5).abs() > 7.5 * 0.30
        );
    } else {
        println!("GATHER_AB_TOTAL thread_cycles_unavailable=true no_cpu_saving_claim=true");
    }
    Ok(())
}

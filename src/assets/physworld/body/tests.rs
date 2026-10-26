use crate::{archive::Catalog, assets::level::LevelResources};
use std::collections::BTreeSet;

#[test]
#[ignore = "requires installed level physics archives"]
fn installed_body_creation_records() {
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let paths: BTreeSet<_> = catalog
        .paths_with_suffix(".bwm")
        .into_iter()
        .filter_map(|path| LevelResources::resolve(&catalog, &path).unwrap().physics)
        .collect();
    let mut counts = [0usize; 3];
    for path in paths {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let world = super::super::parse(&bytes).unwrap();
        let bodies = world
            .body_infos(&bytes)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let poses = world.body_poses(&bytes).unwrap();
        assert_eq!(bodies.len(), poses.len(), "{path}");
        for (body, pose) in bodies.iter().zip(poses) {
            assert_eq!(
                body.position[..3]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                pose.position.map(f32::to_bits),
                "{path}"
            );
            assert_eq!(
                body.orientation.map(f32::to_bits),
                pose.orientation.map(f32::to_bits),
                "{path}"
            );
            for reference in [
                body.shape,
                body.name,
                body.mass_distribution,
                body.local_frame,
            ] {
                assert!(
                    reference < world.items.len(),
                    "{path} item reference{reference}"
                );
            }
            if let Some(attachment) = body.attached_body {
                assert!(
                    attachment < 0 || (attachment as usize) < bodies.len(),
                    "{path} attachment{attachment}"
                );
                counts[2] += 1;
            }
        }
        counts[0] += 1;
        counts[1] += bodies.len();
    }
    assert!(counts[0] > 0 && counts[1] > 0);
    println!("Installed physics worlds/bodies/attachments={counts:?}");
}

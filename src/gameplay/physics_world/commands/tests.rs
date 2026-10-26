use super::*;

#[test]
fn physics_packets_match_complete_executed_router_and_processor() {
    let mut cases = 0;
    for row in include_str!("../../../../tests/fixtures/physics_commands.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let count = words.next().unwrap();
        let mut commands = PhysicsCommands::new().unwrap();
        for _ in 0..count {
            let n = words.next().unwrap() as usize;
            let packet: Vec<_> = words.by_ref().take(n).flat_map(u32::to_le_bytes).collect();
            let decoded = BodyCommand::decode(packet[3], &packet).unwrap();
            // Use the real authored packet encoders where the producer is
            // implemented; raw packets exercise add/activation/no-op routing.
            match decoded {
                BodyCommand::Transform { body, frame, mode } => commands
                    .enqueue_transform(body, frame, mode, false)
                    .unwrap(),
                BodyCommand::Velocity {
                    body,
                    linear,
                    angular,
                } => commands
                    .enqueue_velocity(body, linear, angular, false)
                    .unwrap(),
                _ => {
                    commands.buffer.append_packet(&packet, false).unwrap();
                }
            }
        }
        let mut calls = Vec::new();
        commands
            .buffer
            .dispatch(|group, packet| {
                match BodyCommand::decode(group, packet)? {
                    BodyCommand::Add {
                        body,
                        mode,
                        activation,
                    } => calls.extend([1, body, u32::from(mode), u32::from(activation)]),
                    BodyCommand::Transform { body, frame, mode } => {
                        calls.extend([6, body, u32::from(mode)]);
                        calls.extend(frame.into_iter().flatten().map(f32::to_bits));
                    }
                    BodyCommand::Velocity {
                        body,
                        linear,
                        angular,
                    } => {
                        calls.extend([9, body, 0]);
                        calls.extend(linear.into_iter().chain(angular).map(f32::to_bits));
                    }
                    BodyCommand::Activation { body, deactivate } => {
                        calls.extend([0x1e, body, u32::from(deactivate)])
                    }
                    BodyCommand::Ignored => {}
                }
                Ok(())
            })
            .unwrap();
        let n = words.next().unwrap() as usize;
        assert_eq!(
            calls,
            words.by_ref().take(n).collect::<Vec<_>>(),
            "case{cases}"
        );
        assert_eq!(words.next(), None);
        cases += 1;
    }
    assert_eq!(cases, 256);
}

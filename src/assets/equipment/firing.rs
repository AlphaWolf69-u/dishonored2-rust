//!62ba10's immutable crossbow firing socket. It queries the ordinary Fire
//!scene's first equipment animation and FIRE event, then samples through the
//!same Rig/Clips owner used by the live equipment tree.
use super::*;
use entities::scene_motion::{atom, child, items};

pub(super) fn crossbow_offset(source: &mut SourceRead<'_>, settings: &Ranged, rig: &Rig,
    clips: &mut Clips<'_>, cancelled: Option<&dyn Fn() -> bool>) -> Result<bevy_math::Vec3, String> {
    let Some(scene) = settings.equipment.scenes.get("m_fireMidnight") else { return Ok(bevy_math::Vec3::ZERO) };
    let fields = entities::player::resolved_decl("midnightscene", scene, |path| source.read(path))?;
    let Some(scene) = items(child(&fields, "m_scenes")).map(|wrapper| child(wrapper, "m_object"))
        .find(|scene| atom(child(scene, "m_properties"), "m_name") == Some("Fire")) else { return Ok(bevy_math::Vec3::ZERO) };
    let Some(group_uid) = items(child(&fields, "m_groupsInfo")).find(|group|
        atom(group, "m_name") == Some("PlayerLeftEquipment")).and_then(|group| atom(child(group, "m_UID"), "m_objUID")) else { return Ok(bevy_math::Vec3::ZERO) };
    let Some(group) = items(child(scene, "m_groups")).map(|wrapper| child(wrapper, "m_object"))
        .find(|group| atom(child(group, "m_handle"), "groupUID") == Some(group_uid)) else { return Ok(bevy_math::Vec3::ZERO) };
    let mut alias = None;
    let mut time = 0.0;
    let mut seen_animation = false;
    let mut seen_event = false;
    for track in items(child(group, "m_lTracks")).map(|wrapper| child(wrapper, "m_object")) {
        let keys = child(track, "m_keys");
        match atom(track, "m_trackType").or_else(|| atom(child(track, "m_handle"), "trackType")) {
            Some("EMTT_ANIM") if !seen_animation => {
                seen_animation = true;
                //1957e0 selects the first key only when it starts at zero.
                if let Some(key) = items(keys).next() {
                    if atom(key, "m_startTime").unwrap_or("0") == "0" {
                        alias = atom(child(child(key, "m_keyProp"), "m_object"), "m_AnimName");
                    }
                }
            }
            Some("EMTT_EVENT") if !seen_event => {
                seen_event = true;
                if let Some(key) = items(keys).find(|key|
                    atom(child(child(key, "m_keyProp"), "m_object"), "m_event") == Some("ARK_MDNT_MSG_WEAPON_FIRE")) {
                    time = crate::gameplay::damage::number(key, "m_startTime", 0.0)? * 0.001;
                }
            }
            _ => {}
        }
    }
    let Some(alias) = alias.filter(|alias| !alias.is_empty()) else { return Ok(bevy_math::Vec3::ZERO) };
    //62ba10 first checks that both the model alias and real tag exist.
    if rig.alias(alias).is_none() { return Ok(bevy_math::Vec3::ZERO); }
    let Some(tag) = settings.projectile_attachment_tag.as_deref() else { return Ok(bevy_math::Vec3::ZERO) };
    let Some((joint, offset, _)) = rig.optional_tag_quaternion_in_group("_info", tag)? else { return Ok(bevy_math::Vec3::ZERO) };
    let clip = clips.clip_cancellable(alias, cancelled)?;
    //Virtual+90 is ceil(frameRate), not duration:62ba10 multiplies event
    //seconds by it, then53c8a0 divides by it before24b280's clamped sample.
    let animation = &clips.animations[clip];
    let mut pose = animation.sample(&rig.skeleton.rest, time.min(animation.duration()))?;
    if let Some(root) = pose.first_mut() { root.translation = [0.0; 3]; }
    let matrices = crate::edge_skeleton::pose_matrices(&pose, &rig.skeleton.hierarchy, &rig.root)?;
    let matrix = matrices.get(joint).ok_or("crossbow firing tag joint is outside its actual sampled rig")?;
    Ok(bevy_math::Vec3::new(matrix[0][3], matrix[1][3], matrix[2][3]) + bevy_math::Mat3::from_cols(
        bevy_math::Vec3::new(matrix[0][0], matrix[1][0], matrix[2][0]),
        bevy_math::Vec3::new(matrix[0][1], matrix[1][1], matrix[2][1]),
        bevy_math::Vec3::new(matrix[0][2], matrix[1][2], matrix[2][2])) * offset)
}

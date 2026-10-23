//! `arkMenuSectionOptionAudio` (vtable 0x141b701f8), notes-options.md 4.
use super::{apply_cvar, load_cvar, read_back, Env, Result, Section};

/// `ARK_AUDIOSETTING_*`.
const HEADPHONE: usize = 0;
const SUBTITLES: usize = 7;
const AUDIO_LANGUAGE: usize = 8;
const TEXT_LANGUAGE: usize = 9;

/// The cvar of each setting (Load `FUN_140505560` / Apply `FUN_140505250`);
/// none inverts. AUDIO_QUALITY has no cvar.
fn cvar(index: usize) -> Option<&'static str> {
    Some(match index {
        HEADPHONE => "s_useHeadphoneMode",
        2 => "s_MasterBus_Gain",
        3 => "s_Music_Gain",
        4 => "s_SFX_Gain",
        5 => "s_Voices_Gain",
        6 => "s_Video_Gain",
        SUBTITLES => "s_arkIggySubtitleInitialDisplayLevel",
        _ => return None,
    })
}

/// `FUN_140504f70`: AUDIO_LANGUAGE also needs a current audio language in
/// the language list (`FUN_140504ee0`). The setting is MAIN_MENU-only, so
/// the base rule already hides it in the pause menu; the language list
/// check runs only in the main menu, which the rewrite does not host yet.
pub fn visible(env: &Env, index: usize) -> bool {
    index != AUDIO_LANGUAGE || !env.pause_active
}

pub fn load(env: &mut Env, section: &mut Section, index: usize) {
    if let Some(name) = cvar(index) {
        load_cvar(env, &mut section.decl.settings[index], name, false);
    }
    // AUDIO_LANGUAGE / TEXT_LANGUAGE (`FUN_140504a80` / `FUN_140504b10`)
    // are main-menu settings; the pause menu never shows them.
    let _ = (AUDIO_LANGUAGE, TEXT_LANGUAGE);
}

/// `FUN_1405053d0`.
pub fn apply(env: &mut Env, section: &mut Section) -> Result<bool> {
    let mut any = false;
    for (out, index) in section.elements(env) {
        if read_back(env, section, out, index, true)? {
            any = true;
            if let Some(name) = cvar(index) {
                let s = section.decl.settings[index].clone();
                apply_cvar(env, &s, name, false);
            }
        }
    }
    after_change(env);
    Ok(any)
}

/// `FUN_1405051a0`.
pub fn restore(env: &mut Env) -> Result<()> {
    for name in ["s_useHeadphoneMode", "s_MasterBus_Gain", "s_Music_Gain", "s_SFX_Gain", "s_Voices_Gain", "s_Video_Gain"] {
        let i = env.cvar(name);
        env.console.cvars.reset(i);
    }
    let default = env.console.cvars.list[env.cvar("s_arkIggySubtitleInitialDisplayLevelDefault")].int;
    let i = env.cvar("s_arkIggySubtitleInitialDisplayLevel");
    env.console.cvars.set_int(i, default, true);
    for name in ["ark_savedAudioLanguage", "ark_savedTextLanguage"] {
        let i = env.cvar(name);
        env.console.cvars.set_int(i, 0xffff, true);
    }
    after_change(env);
    Ok(())
}

/// After Apply and Restore: `FUN_140036460` applies a modified subtitle
/// level to the Iggy subtitle player and `FUN_1400a3630` re-applies
/// modified languages. The rewrite has neither subtitles nor a language
/// switch at runtime yet; the cvars carry the values to the next start.
fn after_change(_env: &mut Env) {}

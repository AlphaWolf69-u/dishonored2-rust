# The rewrite folder and launcher

Players download `dishonored2-launcher.exe` into a folder of their choice.
The launcher creates the rewrite folder around itself, downloads the rewrite
source, builds the game with the enabled mods and launches it. The installed
game is only read.

```
DH2Rewrite/
  dishonored2-launcher.exe  launcher and mod manager (prebuilt)
  dishonored2-rust.exe      the game, built by the launcher
  config/
    dishonored2Config.cfg   the game's config format (bindings and cvars)
    rewrite.toml            game_path, rewrite_revision, source_path
  saves/                    profile.bin and SLOTxx folders, the game's formats
  mods/<any folder name>/   one folder per mod (below)
  source/                   the rewrite source (a git clone the launcher keeps)
  cache/target/             build output
  cache/build.txt           what the current game executable was built from
  logs/                     session-*.log (game), build-*.log (launcher)
  modlist.toml              enabled mods by name, in load order
```

`DH2_REWRITE_HOME` selects another folder (development).

**Local source (development):** "Use local source…" stores `source_path` in
`rewrite.toml`. Launch then runs the same cargo build in that checkout as it
is, in its own target folder, and starts the result with this folder's
config, saves and mods. The launcher never checks anything out into a local
source, so mods with patches cannot be enabled with it; resource mods work.
"Use download" returns to `source/`.

Code: `src/platform/home.rs` (layout, imports, settings, mods, resources,
fingerprint), `src/platform/game_folder.rs` (folder and question dialogs),
`src/bin/launcher/` (window `main.rs`, `source.rs`, `toolchain.rs`,
`merge.rs`, `build.rs`, `updates.rs`), `src/assets/archive.rs` (mod resource
overlay).

## Mods

```
mods/<folder>/mod.toml
mods/<folder>/patches/*.patch     changes to the rewrite source (git diff format)
mods/<folder>/resources/...       files at the game's resource paths
```

```toml
name = "Better Stealth"        # the mod's identity: modlist, saves
version = "1.2.0"
author = "..."
rewrite = "v0.4.0"             # the rewrite tag or commit the patches were made against
repository = "https://github.com/user/better-stealth"   # optional, for update checks
description = "..."
saves = "safe"                 # safe | cleanup | permanent
```

- `name`, `version` and `saves` are required; `rewrite` is required when the
  mod has patches. The folder name is free.
- `saves`: `safe` never touches saves; `cleanup` removes its own data from
  saves when disabled; `permanent` means saves started with it need it. The
  launcher asks before disabling a permanent mod.
- Patches are applied in file-name order onto `rewrite`, then that result is
  merged three-way onto the rewrite revision being built, mod by mod in load
  order. A conflict names the mod, the earlier mods and the files.
- A `Cargo.toml` changed by both sides merges by key, not by line
  (`src/bin/launcher/manifest.rs`): additions on both sides are kept
  (dependencies, features, `[[bin]]` entries; arrays merge as sets), a removal
  on one side applies, and a dependency both changed merges field by field
  (its `features` as a set; caret-compatible versions such as `1.2` and
  `1.4` take the higher). Different values for the same key are a conflict
  naming the key, for example `dependencies.rand (version "0.8" and "0.9"
  are not compatible)`.
- A `Cargo.lock` changed by both sides keeps the merged side; cargo adds the
  other side's new packages during the build (at the newest compatible
  versions, not the mod's pinned ones).
- Resources replace or add game resources at run time (a later mod wins);
  they need no build.

## The launcher

1. **Rewrite source:** Download clones
   `https://github.com/AlphaWolf69-u/dishonored2-rust` into `source/` (bundled
   libgit2, no git needed). The revision built is `rewrite_revision`, else the
   newest release tag, else `main`. Check for updates fetches; a newer release
   tag is offered as "Update to vX", a moved branch as "Update to main".
2. **Build tools:** cargo (PATH or `~/.cargo/bin`) and the MSVC C++ build tools
   (found with `vswhere`). Install buttons run winget in a console window, or
   open the download pages without winget.
3. **Game folder:** found as the game does (`game_path`, Steam), or chosen with
   Browse… (any store's installation).
4. **Mods:** checkboxes and ▲/▼ load order (`modlist.toml`), save-safety
   badges, update notices (the mod repository's newest version tag), problems.
5. **Launch:** if the rewrite commit or the enabled mods' names, versions or
   patches changed since the last build (`cache/build.txt`), the mods are
   merged into `source/` and `cargo build --release --features game --bin
   dishonored2-rust` runs (output `cache/target`, log `logs/build-*.log`); the
   game executable is copied next to the launcher and started.
6. **Saves and config** are imported only on request (Import saves…, Import
   config…; or `dishonored2-rust.exe --import-saves [folder]`).

## Setup fingerprint

Each game session log records a BLAKE3 fingerprint of the rewrite version,
the installed game's four `game*_patch.index` files, and every enabled mod
(name, version, patches, resource paths and contents) in load order.

## Not covered yet

- Saves do not record their mods yet (the rewrite does not write saves yet),
  so the permanent-mod warning cannot list affected saves, and `cleanup`
  mods have no launcher step to run their cleanup.
- Downloading mod updates (only the check exists).
- Mod profiles.
- Loose `base/` files the engine reads directly (`default.cfg`,
  `default_keybinds.cfg`, `SKUConfig.txt`) cannot be replaced by mods.

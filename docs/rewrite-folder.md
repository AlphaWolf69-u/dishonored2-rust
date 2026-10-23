# The rewrite folder and launcher

The rewrite keeps everything it writes in one folder next to its executable
(`DH2_REWRITE_HOME` selects another one). The installed game is only read.
The first launch creates the layout below.

```
DH2Rewrite/
  dishonored2-launcher.exe  launcher and mod manager
  dishonored2-rust.exe
  config/
    dishonored2Config.cfg   the game's config format (bindings and cvars)
    rewrite.toml            rewrite settings: game_path
  saves/                    profile.bin and SLOTxx folders, the game's formats
  mods/<id>/                one folder per mod (see below)
  overrides/                simple resource swaps
  cache/
  logs/                     session-*.log
  modlist.toml              enabled mods, in load order
```

Code: `src/platform/home.rs` (layout, import, settings, mods, fingerprint),
`src/platform/game_folder.rs` (folder prompt), `src/assets/installation.rs`
(game folder lookup), `src/assets/archive.rs` (resource overlay).

## First launch

- **Game folder:** `DH2_GAME_BASE` if set, else `game_path` in
  `config/rewrite.toml`, else a Steam installation (AppID 403640). If none is
  found, a folder dialog asks for the installation (the folder with `base`, or
  `base` itself) and the choice is saved to `rewrite.toml`. Any store's
  installation with the standard `base/game*_patch.index` files works.
- **Config and saves are imported only on request**, never guessed from the
  original's location. A new folder starts with the game's defaults
  (`default.cfg`, the cvar defaults).
  - Launcher: **Import saves…** (a folder with `profile.bin` and the `SLOTxx`
    folders; files already in `saves/` are kept) and **Import config…** (a
    folder with `dishonored2Config.cfg`; it replaces the rewrite's).
  - Command line: `dishonored2-rust.exe --import-saves [folder]` (a dialog
    when no folder is given); the game does not start.
- The original's files are never written.

## Launcher

`dishonored2-launcher.exe` (built with `--features launcher`, Slint) shows the
game folder (Browse…), the installed mods with enable checkboxes and ▲/▼ load
order (written to `modlist.toml`; invalid or missing mods are listed with the
reason), the override count, the imports, folder shortcuts, the setup
fingerprint, and Launch, which starts `dishonored2-rust.exe` from the same
folder on the same rewrite folder. Code: `src/bin/launcher.rs`.

## Resource overrides

A file in `overrides/` replaces the game resource with the same path, for
example `overrides/strings/english_m.lang` or
`overrides/generated/decls/<type>/<name>.decl`. New paths add resources.
Overrides win over every mod.

## Mods

```
mods/<id>/mod.toml        id = "<id>", name = "...", version = "...", description = "..."
mods/<id>/resources/...   files at the game's resource paths
```

`modlist.toml` enables mods in load order: `enabled = ["a", "b"]`; a later
mod's file wins over an earlier one's. A mod whose `mod.toml` is missing, or
whose `id` differs from its folder name or has no `version`, is skipped and
reported in the session log.

## Setup fingerprint

Each session log records a BLAKE3 fingerprint of the rewrite version, the
installed game's four `game*_patch.index` files, and every enabled mod and
override (ids, versions, resource paths and contents, in load order). Equal
fingerprints mean the same game data and mod setup.

## Not covered yet

- Loose `base/` files the engine reads directly (`default.cfg`,
  `default_keybinds.cfg`, `SKUConfig.txt`) are not overlaid.
- Patching instead of replacing (decl fields, string-table keys) and code mods.
- The text language on non-Steam installations: the Steam language lookup
  fails, so the saved `ark_savedTextLanguage` or English applies.
- Saves are not yet tagged with the fingerprint.

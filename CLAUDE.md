# Blacksmith — notes for Claude

Blacksmith, 工程文件医生 — "the project doctor for FL Studio and Ableton Live" (owner's branding,
Nov 2026; no longer pitched as a VST2→VST3 upgrader). Logo: `assets/logo.svg` (anvil + green
cross); `tools/icons.sh` renders `.icns` / `.ico` / window icon from it.

Rust + iced 0.14 desktop app with one entry, 检查工程: drop one FL Studio project (.flp) or
Ableton Live set (.als) in (one at a time — owner's call), see every plugin they use — format, whether FL on this computer can load it (by format + ID,
against FL's own plugin database), what can be done (upgrade VST2 → VST3, move a wrong VST3 ID to
the installed plugin, or point it at one the user picks) — tick what to do, press 应用 to save
`<name> (修复).flp` next to the original. Flow modelled on livesaver (owner's call, Nov 2026).
UI text is Chinese. Owner wants it fast,
light and ergonomic; neutral colours following system light/dark; springy
(ease-out-back) motion.

## Layout

- `src/flp.rs` — lossless FLP event reader/writer + FL's `id u32 · size u64 · data` record lists.
- `src/check.rs` — what every project format shares: `Plugin` (format + ID + name), `Status`,
  `Fix`, `Finding`, user `Choices` (Off / On / Pick an installed VST3, each with `reset`), `decide`,
  `pending`. Only verified fixes (official rules) default on; unverified ones (generic upgrade, rename
  by name, picks) must be ticked, with 保留设置 (keep state; may crash the host) or 重置设置 (drop
  state: FL record 53 removed, Live `<ProcessorState />`; plugin opens with defaults).
- `src/document.rs` — `Document` = FL `.flp` or Live `.als`: read, host, analyse, apply, bytes.
  The app and CLI only talk to this.
- `src/fl.rs` — FL projects: per-instance status/fix (`analyse`) and `apply`, using `convert.rs`
  and `repair.rs`.
- `src/als.rs` — Live sets: gunzip XML, find `<VstPluginInfo>` / `<Vst3PluginInfo>` blocks, change
  only the `<Fields.N>` digits of a VST3's `<Uid>` (both copies), gzip. Phase 1: VST3 IDs only.
- `src/convert.rs` — VST2 → VST3 for one instance (`upgrade_instance`, optional user-picked
  target): VST2 state parsing, VST3 record building, FXB wrapping.
- `src/plugins.rs` — 官方支持 rules (Serum, OTT). Each rule: VST2 FourCC, VST3 class ID, built-in template
  (`assets/*.bin`), state transform, and a note shown in the app.
- `src/upgrades.rs` — 升级插件: rules moving a plugin to a different, newer product (Kontakt →
  Kontakt 8, LABS → Splice INSTRUMENT; later Serum → Serum 2…). `deprecated` marks a discontinued
  source plugin: `Status::Deprecated` (已停产) when installed, listed under 停产插件. The FL template
  is optional (`None`: Live only).: both class IDs, the target's FL template
  (`assets/<plugin>-vst3-template.bin`), state transform, `verified`. Shown as `Fix::Migrate`
  (always opt-in) in FL (`repair::move_vst3`: new ID, name, path, param table) and Live
  (`als::upgraded`: Uid, info `<Name>`, `<ProcessorState>` through the transform). A new upgrade
  needs a project with both plugins holding the same sound.
- `src/vst2to3.rs` + `assets/vst2to3.tsv` (the rules as data, one row per plugin) — VST2 → VST3 of
  the same plugin in Live sets: per VST2 ID the VST3 class ID,
  Live's (inputs, outputs, DeviceType) and how the VST3 holds the chunk (`State::AsIs`, `VstW`,
  `LengthPrefixed`, `FabFilter`). Shown as `Fix::Upgrade` with `How::Matched` (unverified) or
  `How::Known`. Which project each rule (here and in `plugins.rs` / `upgrades.rs`) came from:
  `docs/plugins.md` — add a row with every new rule.
- `src/labs.rs` — LABS state → Splice INSTRUMENT: the JUCE binary XML (`VC2!`) `<Labs>` preset handed
  over as is; INSTRUMENT reads it itself (verified in Live, Nov 2026).
- `src/vst3.rs` — Steinberg's derived class ID and default VST3 paths.
- `src/livedb.rs` — reads Live's plugin database (`Live Database/Live-plugins-<n>.db`, SQLite via
  rusqlite, read-only): `device:vst:<kind>:<id>?n=…` / `device:vst3:<kind>:<FUID>`.
- `src/fldb.rs` — reads FL's own plugin database on this computer (`.nfo` text files only). Owner's
  call (Nov 2026, after looking at livesaver): use what FL scanned, as Live tools use Live's DB;
  still never scan plugin folders or load plugins (installs are often messy, pirated or mismatched).
- `src/main.rs` — app state and update (sidebar: 检查工程 / 已安装插件 / 官方支持), CLI
  `blacksmith a.flp folder …` (check only, writes nothing) and `--apply` (save the default fixes).
- `src/view.rs` — all drawing: sidebar, top bar (title + page action), steps + drop zone when a
  page is empty, then tables (插件 / 工程 tabs). Look modelled on livesaver: near-monochrome, ink
  primary button, status as coloured mark + words, hairline tables; springy reveals kept.
- `src/style.rs` — palette via `p()`, widget styles. `src/platform.rs` — macOS dark-mode read.
- `src/folders.rs` — standard plugin folders per OS (Mac `/Library` + `~/Library/Audio/Plug-Ins/{VST3,VST}`,
  Windows Common Files VST3/VST2, Program Files VSTPlugins…), VST3 class IDs from `moduleinfo.json`
  (tiny JSON5 parser, no crates). Only ~55 of 206 bundles on the owner's Mac have one; VST2/CLAP IDs
  need loading the plugin, which we don't.
- `src/library.rs` — one `Library` per `Host` (`library::get(Host::Fl | Host::Live)`), all built by
  the same merge: the host DB (`HostDb` from `fldb.rs` / `livedb.rs`) decides "installed", minus entries whose file is gone; folders fill IDs the host didn't record (only
  for files the host lists) and report plugins the host hasn't scanned; with no host DB the folders
  are the list (+ built-in rows whose bundle is on disk). `assets/known_ids.tsv` (built-in) also
  pairs FourCCs for converting. `Scan` holds the stats for the 已安装插件 page. Tests use the
  built-in table only.
- `tools/plugindb.py` — dev tool that updates the table from FL's plugin database.
- `src/repair.rs` — one VST3 instance (`repair_instance`, `retarget`): an official rule's relink
  (aliases, e.g. old OTT) or the library ID of the same name + vendor.
- `src/dump.rs` — `--dump` developer view / conversion check.
- `src/arch.rs` — Apple Silicon only: whether an installed plugin's binary lacks arm64 (Mach-O
  header read from the file, never loaded) → `Status::Rosetta` (仅 Intel · 需 Rosetta). The library
  keeps the Intel-only file paths (`Library::intel_only`, `vst2_intel_only`, `vst3_intel_only`).
  Owner's Mac (Nov 2026): 96 of 685 bundles Intel-only (Omnisphere, DUNE3 VST2 — its VST3 is
  universal —, PITCHMAP…). FL's `.nfo` also says `ps_file_arch_N=Intel`; not used.

Dev CLI: `blacksmith --dump a.flp` lists VST instances; `blacksmith --dump orig.flp resaved.flp`
compares the original with FL's re-save of the converted copy (each loaded VST3 writes its own
state on save, so this shows whether the settings carried over).
`blacksmith --dump x.als` lists a Live set's plugins with their states and compares instances of
the same VST3 (`FabF` value by value, `VstW` chunks inside, JUCE tails noted): put a reference VST3
next to the VST2 in the test set, convert, re-save in Live, dump — the pairs should match.
`--dump a.als b.als` pairs instances in order.

`blacksmith --apply-all x.flp` saves every available change, unverified ones too, keeping settings
(dev aid for producing test files). `blacksmith --library` prints both hosts' libraries (and where they came from).

The project is analysed off the UI thread. The CLI searches folders for `.flp`, skipping `Backup`
folders and Blacksmith's own `(修复)` (and older `(VST3)`) copies.
Installed = class ID in FL's DB or one of its `ps_compatibility` IDs; installed VST2 = `magic`
in FL's DB (unknown without the DB). FL lists some VST3s without a class ID (scan unfinished, `tip= /  bit`:
Kontakt 7/8, SWAM, MetaTune… 22 on the owner's Mac; no `moduleinfo.json` either) and finds them
by file, so a VST3 instance whose bundle file name or name matches one of those counts as installed. An installed VST2 counts as opening; it's listed under
需要处理 only when an installed VST3 can replace it.

Debug env vars: `BLACKSMITH_FLDB=<Installed folder>|none`, `BLACKSMITH_LIVEDB=<file>|none`, `BLACKSMITH_PLUGIN_DIRS=<path list>`, `BLACKSMITH_DROP=<file>`, `BLACKSMITH_PAGE=supported|library`, `BLACKSMITH_DARK=1`.

## FL Studio format facts (verified on real projects)

- Plugin data is event 213: `u32 version` + records. The version is the wrapper format, not the
  plugin type: 10 in FL 12–21 saves, 12 in FL 24–25, 8/9 in between (6–12 accepted). Kept on
  write. Native plugins use 213 too, with arbitrary bytes — record parsing must stay checked. 50 flags (VST3: first u32 = 8),
  51 FourCC (VST2 only, LE), 52 class ID (16 bytes, COM byte order), 54 name, 55 path,
  56 vendor, 57 = 0x960 on every VST2 seen (not the parameter count), 53 state.
- FL sometimes stores record 52 on a **VST2** instance (when its plugin manager pairs the
  formats, e.g. Vital). Use it; don't duplicate it.
- VST2 state (53): 21-byte header; byte 4 bit 3 set (0x0d) = chunk: u64 size at 9..17, chunk
  from 21. Clear (0x05) = parameters: `u32 n, n × f32, u32, program name\0, …` (FabFilter Pro-R).
- VST3 state (53): `u32 1` + records: 1 = 64-byte header (`01` then zeros), 3 = component state,
  4 = parameter-ID table (`u32 n, n × u32`).
- Serum: VST3 reads its VST2 chunk unchanged; class ID follows Steinberg's rule
  (`XTSVsfsXerum…`). Conversion verified in FL Studio 2025 by the owner.
- Generic path: class ID from VST3 instance in project → `known_ids.tsv` table → FL's record →
  Steinberg's rule (badged 推导 ID). Path uses the table's .vst3 file name when known. State handed over as `VstW` + FXB (`FBCh` chunk bank, or `FxBk` with one
  `FxCk` program), per the VST3 SDK's `tryVst2StateLoad`. Vital worked this way.
- Generic path checked in FL 2025 via `--dump` (Nov 2026): Pro-Q 3 takes the FBCh chunk (same
  `FFBS` data re-saved); Disperser and Decapitator load but re-save in their own format (not
  comparable automatically); OTT (parameter-based VST2) loads but ignores the `FxBk` bank —
  Depth went 0.614 → 1.0 (default). Now an official rule: OTT's VST3 state is 20 LE f32s (the
  VST2's 19 + one new, 0), parameter table 0..=19 + 'prog'.
  Verified: after an FL re-save `--dump` shows all 19 parameters equal.
- Older OTT VST3 (seen in a project from the web, FL wrapper v11, Windows): class ID
  `XESVTfoTtt…` ('VSE' not 'VST'), 19 floats, table 0..=18 + 'prog'. FL finds OTT by name but
  shows "This plugin failed to load" (not "missing") — so "found" ≠ "loads". Rules list such IDs as `aliases`; `repair` relinks them (badge 修复 · 官方支持).
- The library (`known_ids.tsv`) holds every VST3 FL scanned (296 on the owner's Mac: 226 with a
  VST2, 70 without, fourcc `-`). Repair's name match: lower-case alphanumerics, vendor prefix and
  `x64` suffix dropped. Seen on the owner's projects: Deelay saved with JUCE codes `Sixt/Effy`,
  installed build uses `Manu/Wnne` — repair moves 21 instances; not yet checked in FL. On re-save FL also fixes record 55 to the real VST3 path.
- FabFilter does **not** follow Steinberg's ID rule: derived IDs for Pro-Q 3 / Pro-R fail to
  open. Real IDs are now in `known_ids.tsv`; whether FabFilter's VST3 accepts the FXB state is
  not yet checked in FL.

## FL's plugin database (`Presets/Plugin database/Installed/`)

- One `.nfo` (UTF-8 with BOM, `key=value`) per plugin; `ps_file_*_N` per file. VST3 entries:
  `ps_file_guid_N` = real class ID (`{D1-D2-D3-…}`, → record 52 by byte-swapping D1/D2/D3).
  VST2 entries: `ps_file_magic_N` = FourCC as a big-endian integer; their `guid` is only
  Steinberg's derived ID (wrong for FabFilter).
- `ps_file_plugclass`: VST2 0 effect / 4 instrument, VST3 7 effect / 8 instrument. Same value as
  the first u32 of record 50. FL saves VST3 effects with 7; Blacksmith writes 8 for every VST3.
  FL 2025 loads converted effects with either 7 or 8 (checked on Lifestyle Lead, Nov 2026).
- FL puts both formats in one `.nfo` mostly when the VST3 follows Steinberg's rule; plugins that
  don't (FabFilter, JUCE `ABCDEF01-9182-FAEB-…` IDs) get separate `.nfo`s.
- The `.fst` next to it holds only records 2/50/51or52/54/(55)/56 — no state, header or
  parameter table, so it is not a full template.
- `tools/plugindb.py` reports VST2↔VST3 candidates from it; `--tsv` merges them into
  `assets/known_ids.tsv` (226 plugins from the owner's Mac, Nov 2026). Running it on another
  machine adds that machine's plugins and keeps the rest; `--replace` starts over.

## Ableton Live sets (verified on the owner's 303 sets, Nov 2026)

- `.als` = gzip of XML (`<Ableton … Creator="Ableton Live 12.2.1">`). Plugins: `<PluginDevice>` →
  `<PluginDesc>` → `<VstPluginInfo>` (`UniqueId` decimal, `PlugName`, `Path`; state `<Buffer>` hex)
  or `<Vst3PluginInfo>` (`Name`; `<Uid><Fields.0..3>` = FUID as 4 big-endian words written as
  signed decimals, `ABCDEF01` → -1412567295; repeated inside `<Vst3Preset>`; state
  `<ProcessorState>`). No vendor is stored, so name matching is by name only.
- Live's DB on the owner's Mac: 804 rows, 288 distinct VST3s (has Kontakt 8's ID, unlike FL's).
- Seen: Ample Guitar SC/VC saved with older `VSTA…` IDs; the installed builds use JUCE IDs.
  Relinking them kept the old state (`VstW`+FXB around XML root `AEG_STATE` 3.2.0) and **Live crashed**
  in AGSC 3.7.0 `AAGParameterList::loadState` (null element). Hence unverified fixes default off and
  the reset option. With 重置设置 (`<ProcessorState />`) the same set opens in Live 12.2.1 (checked
  by the owner, Nov 2026). A proper Ample rule needs a set saved by the new build to compare.
- VST2 → VST3 in sets: `<VstPluginInfo>` rebuilt as `<Vst3PluginInfo>` (`als::vst3_info_from_vst2`)
  with `<Buffer>` → `<ProcessorState>` through a `vst2to3.rs` rule (or an upgrade). Live binds a
  small plugin's first parameters itself (`<ParameterId>` = VST2 index); the VST3 numbers them
  differently, so unused bound slots are unbound (`als::unbind`) and a device with an automated
  (`<PointeeId>` of its Automation/ModulationTarget) or MIDI-mapped (`<KeyMidi>`) slot is refused.

## Kontakt (6) → Kontakt 8 (test project `test-projects/kontakt vs kontakt 8/`)

- Both VST3: Kontakt `NTSV$ikDontakt` (state says 6.7.0.0), Kontakt 8 `NTSVKik8ontakt 8` (8.5.0.0).
  States are NI `hsin` containers with the same layout but compressed contents (0.4% bytes shared),
  so no byte-level conversion: the old state goes to Kontakt 8 as is. Verified by the owner in
  FL Studio 2025 (Nov 2026): opens with the same instrument and sound.
- FL's DB lists Kontakt 8 without a class ID, so `Upgrade::installed_in` also accepts a by-file match.
- Record 32 (I/O layout) differs (708 vs 384 bytes); the old one is kept.
- Live: Kontakt VST3 → Kontakt 8 also verified by the owner in a set.
- Kontakt **VST2** (`Ni$D`, Kontakt 5/6): its chunk is the same `hsin` container, so it goes to
  Kontakt 8 as is (rule `Source::Vst2`; verified by the owner in Live sets, Nov 2026, not yet in FL). In a Live set the `<VstPluginInfo>` is rebuilt as a
  `<Vst3PluginInfo>` (`als::vst3_info_from_vst2`, modelled on a real Kontakt 8 block): window pos,
  on/off, overwrite number and preset Id carried over, `<Buffer>` text verbatim as
  `<ProcessorState>`, `<BranchDeviceId>` → `device:vst3:…`. Refused when any `<ParameterId>` in the
  device isn't -1 (bound parameters/automation would point at VST2 numbering). Seen on the owner's
  `gcse Project/version 2.als`: 46 Kontakt 6.6 VST2 instances, all slots unbound.

## Plugin sources — why not just scan folders

Checked on the owner's Mac (Nov 2026): FL's DB listed 607 plugin files, all still on disk; the
folders held 583 VST/VST3, only 2 not in FL's DB. FL rewrites the whole DB when the user rescans.
Hosts find plugins by format + ID, and folders mostly can't give IDs without loading plugins, so
the host's DB stays primary. For Ableton (.als, planned): read Live's plugin DB (SQLite) as another
host source; the merge, check flow and UI are meant to be reused.

## Adding an official plugin

Get the VST3 class ID from `tools/plugindb.py`. For the state transform, template header and
parameter table, ask for a project with both the VST2 and VST3 instance loaded with the same
preset. Compare records 52/53 between them, decide the state transform, extract the VST3
instance's records (state record 3 emptied) into `assets/<plugin>-vst3-template.bin`, add a `Rule`.

## Building

- `cargo test`, `cargo build --release`.
- Windows exe from Linux without rustup targets: build std from source with
  `RUSTC_BOOTSTRAP=1 cargo build --release -Zbuild-std=std,panic_abort --target x86_64-pc-windows-gnu`
  and `linker = "x86_64-w64-mingw32-gcc"` (needs rust-src matching the compiler).
- CI (`.github/workflows/release.yml`) builds Windows (.zip) + macOS (.dmg via `tools/dmg.sh`: app + Applications link, Finder layout set by AppleScript, skipped without a GUI) and releases on push to main. The
  release notes list every commit subject since the previous `v*` tag, so write subjects for users
  (what changed, not how). Curated notes can replace them afterwards with `gh release edit`.

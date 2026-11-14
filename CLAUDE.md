# Blacksmith — notes for Claude

Rust + iced 0.14 desktop app: drop FL Studio projects (.flp) in, see every VST
plugin with a badge, press 转换 to save `<name> (VST3).flp` next to the original
with VST2 instances swapped for VST3. UI text is Chinese. Owner wants it fast,
light and ergonomic; neutral colours following system light/dark; springy
(ease-out-back) motion.

## Layout

- `src/flp.rs` — lossless FLP event reader/writer + FL's `id u32 · size u64 · data` record lists.
- `src/convert.rs` — the upgrade: VST2 state parsing, VST3 record building, FXB wrapping, `Report`.
- `src/plugins.rs` — 官方支持 rules (Serum). Each rule: VST2 FourCC, VST3 class ID, built-in template
  (`assets/*.bin`), state transform, and a note shown in the app.
- `src/vst3.rs` — Steinberg's derived class ID and default VST3 paths. Never scans the machine
  (owner's call: installs are often messy, pirated or mismatched).
- `src/main.rs` — app (sidebar: 转换工程 / 官方支持), CLI mode `blacksmith a.flp …`.
- `src/style.rs` — palette via `p()`, widget styles. `src/platform.rs` — macOS dark-mode read.

Debug env vars: `BLACKSMITH_DROP=<file>`, `BLACKSMITH_PAGE=supported`, `BLACKSMITH_DARK=1`.

## FL Studio format facts (verified on real projects)

- Plugin data is event 213: `u32 kind` (10 = VST) + records. 50 flags (VST3: first u32 = 8),
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
- Generic path: class ID from FL's record → VST3 instance in project → Steinberg's rule
  (badged 推导 ID). State handed over as `VstW` + FXB (`FBCh` chunk bank, or `FxBk` with one
  `FxCk` program), per the VST3 SDK's `tryVst2StateLoad`. Vital worked this way.
- FabFilter does **not** follow Steinberg's ID rule: derived IDs for Pro-Q 3 / Pro-R fail to
  open. Needs an official rule (real class ID from a project containing the VST3).

## Adding an official plugin

Ask for a project with both the VST2 and VST3 instance loaded with the same preset. Compare
records 52/53 between them, decide the state transform, extract the VST3 instance's records
(state record 3 emptied) into `assets/<plugin>-vst3-template.bin`, add a `Rule`.

## Building

- `cargo test`, `cargo build --release`.
- Windows exe from Linux without rustup targets: build std from source with
  `RUSTC_BOOTSTRAP=1 cargo build --release -Zbuild-std=std,panic_abort --target x86_64-pc-windows-gnu`
  and `linker = "x86_64-w64-mingw32-gcc"` (needs rust-src matching the compiler).
- CI (`.github/workflows/release.yml`) builds Windows + macOS and releases on push to main.

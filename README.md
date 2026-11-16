# Blacksmith

Drop an FL Studio project (`.flp`) or Ableton Live set (`.als`) in and see, for every plugin it
uses, whether FL or Live on this computer can open it — and fix what it can't: upgrade VST2 plugins to VST3 with the same
sound and settings, or point a plugin saved under an ID you don't have at the plugin you do have.
Nothing is written until you press 应用; the result is saved next to the project as
`<name> (修复).flp`, and the original is never touched.

## 检查工程

One table, one row per plugin in the project:

| Column | |
|---|---|
| 状态 | ✓ 已安装 (FL finds it here, by format and ID) · ⚠ ID 对不上 (that VST3 ID isn't installed, but a plugin of the same name and vendor is) · ✕ 未安装 · VST2 (installed or not) |
| 可以做什么 | 升级到 VST3 · 改成已装的插件 · 选择已装的插件… (pick any installed plugin yourself) |
| 官方支持 / 未验证 | Official rules (`src/plugins.rs`, currently Serum and OTT) are checked in FL: the plugin loads with its settings. Everything else is marked 未验证: FL finds the plugin, but whether it reads the state hasn't been tried. |

Tick or untick each row; fixes that would leave a plugin FL can't find (an upgrade to a VST3 that
isn't installed) start unticked.

## Plugin library

Each host gets its own library. What counts as installed comes from, in order:

1. **The host's own plugin database** on your computer — FL Studio's
   (`Documents/Image-Line/FL Studio/Presets/Plugin database/Installed`, text files) or Live's
   (`Ableton/Live Database/Live-plugins-1.db`, read-only) — exactly what that host can load.
   Entries whose file is gone are ignored.
2. **The standard plugin folders** (`/Library/Audio/Plug-Ins/VST3` etc. on a Mac,
   `Common Files\VST3` etc. on Windows): what is on disk, plus the class IDs a VST3 states in its
   `moduleinfo.json`. They fill in IDs FL didn't record and show plugins FL hasn't scanned yet.
3. A built-in list (`assets/known_ids.tsv`) of VST2 → VST3 pairs, used for converting.

Only files are read; no plugin is loaded. The 已安装插件 page shows what each source found.

```
blacksmith song.flp folder/        # no window: check only, writes nothing
blacksmith --apply song.flp        # …and save the default fixes as "song (修复).flp"
blacksmith --library               # the plugin library in use
blacksmith --dump song.flp         # every VST instance (IDs, paths, state)
blacksmith --dump song.flp "song (修复) re-saved in FL.flp"   # did the settings carry over?
```

In Live sets, Blacksmith currently fixes VST3 IDs (to the installed plugin of the same name, or one
you pick). VST2 → VST3 for Live is not done yet: Live stores plugin state differently.

## How it works

FL stores each wrapped plugin as a list of records: identity (VST2 unique ID or VST3 class ID),
binary path, and saved state. FL finds a plugin by format and ID, never by name.

- **Upgrading** a VST2 swaps the identity and path for the VST3's and moves the state into FL's
  VST3 layout (header · component state · parameter-ID table). Official rules have an exact
  template and state conversion. Otherwise the state is handed over the way the VST3 SDK's
  `tryVst2StateLoad` expects (a `VstW` header plus an FXB bank), with VST3 parameter ID = VST2
  parameter index so automation keeps its targets. The VST3's ID comes from a VST3 instance in
  the project, the plugin library, FL's own record, or Steinberg's naming rule, in that order.
- **Changing an ID** points a VST3 instance at the installed plugin's class ID and keeps its
  state; official rules also convert the state of older builds (OTT).

## Build

```
cargo build --release
```

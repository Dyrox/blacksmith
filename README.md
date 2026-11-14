# Blacksmith

Drop an FL Studio project in, get a copy with its VST2 plugins swapped for
their VST3 versions — same sounds, same settings. The original is never
touched: the result is saved next to it as `<name> (VST3).flp`.

Drop projects in (or use 选择文件…): Blacksmith first lists every VST plugin in
each project, then writes the upgraded copies when you press 转换:

| Badge     | Meaning |
|-----------|---------|
| 官方支持   | A plugin with its own tested rule (`src/plugins.rs`). Currently: Serum. |
| 通用转换   | Generic conversion following Steinberg's VST2→VST3 spec; not verified for this plugin, check it in FL. |
| 已是 VST3  | Already VST3, left alone. |
| 跳过       | Couldn't be converted (reason shown). |

```
blacksmith song.flp other.flp   # no window, prints the same report
```

## How it works

FL stores each wrapped plugin as a list of records: identity (VST2 unique ID
or VST3 class ID), binary path, and saved state. Converting an instance means
swapping the identity and path for the VST3 ones and moving the state into
FL's VST3 layout (header · component state · parameter-ID table).

**Known plugins** (`src/plugins.rs`) have an exact class ID, a built-in
template taken from a real FL project, and a state conversion verified in FL.
Serum's VST3 reads its VST2 chunk unchanged.

**Everything else** goes through the generic path, built on the VST3 SDK's
rules for replacing a VST2 plugin:

1. *Which VST3?* A VST3 instance of the same plugin already in the project;
   otherwise an installed `.vst3` that declares it replaces the VST2 ID
   (`moduleinfo.json` → Compatibility) or uses Steinberg's derived class ID
   (`"VST"` + VST2 ID + first 9 letters of the lower-case name); otherwise one
   with the same name, assuming that derived ID.
2. *State:* the VST2 chunk is handed over the way Cubase does and the SDK's
   `tryVst2StateLoad` expects — a `VstW` header plus an FXB chunk bank.
3. *Parameters:* VST3 parameter ID = VST2 parameter index, as the spec
   requires, so automation keeps pointing at the same controls.

Only chunk-based VST2 state is handled for now; other plugins are skipped.

## Build

```
cargo build --release
```

# Blacksmith

Drop an FL Studio project in, get a copy with its VST2 plugins swapped for
their VST3 versions — same sounds, same settings. The original is never
touched: the result is saved next to it as `<name> (VST3).flp`.

Drop projects in (or use 选择文件…): Blacksmith first lists every VST plugin in
each project, then writes the upgraded copies when you press 转换:

| Badge     | Meaning |
|-----------|---------|
| 官方支持   | A plugin with its own tested rule (`src/plugins.rs`). Currently: Serum. |
| 通用转换   | Generic conversion following Steinberg's VST2→VST3 spec; not verified for this plugin, check it in FL. `· 推导 ID`: the VST3 ID was derived, not known. |
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

**Everything else** goes through the generic path. It reads nothing but the
project — VST2/VST3 installs are often messy or mismatched, so the machine is
never scanned:

1. *VST3 class ID:* FL's own record of it (FL stores one on a VST2 instance
   when its plugin manager pairs the two formats); else a VST3 instance of the
   same plugin in the project; else Steinberg's rule (`"VST"` + VST2 ID + first
   9 letters of the lower-case name), shown as **通用转换 · 推导 ID** because some
   vendors (e.g. FabFilter) don't follow it.
2. *State:* handed over the way Cubase does and the SDK's `tryVst2StateLoad`
   expects — a `VstW` header plus an FXB bank: a chunk bank for chunk-based
   plugins, or one program of parameter values for the rest.
3. *Parameters:* VST3 parameter ID = VST2 parameter index, as the spec
   requires, so automation keeps pointing at the same controls.

The list of officially supported plugins and what each badge means is in the
app (官方支持列表).

## Build

```
cargo build --release
```

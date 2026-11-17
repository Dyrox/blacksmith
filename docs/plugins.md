# Plugin support: where each rule came from

Every plugin-specific rule in Blacksmith comes from a project or set that holds the plugin in
both forms, with the same sound. This file records which project each rule was taken from and
what was checked. Test projects live in `test-projects/`, which git ignores, so keep them on
disk.

Rules are kept in three places:

- `src/plugins.rs`: 官方支持 for FL.
- `src/upgrades.rs`: 升级插件, moving to a newer product.
- `assets/vst2to3.tsv`: VST2 → VST3 of the same plugin in Live sets, one row per plugin (the
  state conversions it names are in `src/vst2to3.rs`).

| Status | Meaning |
|---|---|
| verified | Opened in the host after converting, with the same sound and settings. |
| byte-identical | The converted state is exactly what the host saved for the plugin's own VST3. |
| unverified | Not opened in the host yet. |

## FL Studio (`.flp`)

| Plugin | Rule | Project | State conversion | Status |
|---|---|---|---|---|
| Serum | VST2 → VST3 (`plugins.rs`) | the owner's FL project with Serum (file name not recorded) | chunk as is | verified, FL Studio 2025 |
| OTT | VST2 → VST3 (`plugins.rs`) | the owner's FL project with OTT VST2 + VST3 | 19 f32 → 20 f32 (+1 new, 0), param table 0..=19 + `prog` | verified: FL re-save, `--dump` shows all 19 parameters equal |
| OTT (older VST3, `XESV…` ID) | relink alias (`plugins.rs`) | a project from the web (Ariiol, Windows, FL wrapper v11) | 19 → 20 f32 | verified: the repaired project opens |
| Kontakt (6) VST3 → Kontakt 8 | 升级插件 (`upgrades.rs`) | `test-projects/kontakt vs kontakt 8/kontakt vs kontakt 8.flp` | `hsin` state as is, Kontakt 8's FL template | verified, FL Studio 2025 |

## Ableton Live (`.als`)

| Plugin | Rule | Project | State conversion | Status |
|---|---|---|---|---|
| Kontakt (6) VST3 → Kontakt 8 | 升级插件 | the owner's set (file name not recorded) | as is | verified, Live 12 |
| Kontakt 5/6 VST2 → Kontakt 8 | 升级插件 | `gcse Project/version 2.als` (46 instances), then another set | `<Buffer>` → `<ProcessorState>` as is | verified, Live 12 |

### From `test-projects/supply 1.als` (Live 12.2, Nov 2026)

This set holds every plugin below as both VST2 and VST3. All rules are rows in `assets/vst2to3.tsv`.
Verified (Nov 2026): the owner opened the converted copy in Live 12.2 and saved it as
`test-projects/supply 1 (修复) 2.als`. `blacksmith --dump` shows every converted VST3's state
byte-identical to the plugin's own VST3 next to it, JUCE tails included. All 19 rules are on by
default.

| Plugin | VST2 ID | State conversion (`State::…`) | Before re-save |
|---|---|---|---|
| FabFilter Pro-Q 3 | `FQ3p` | `FabFilter`: `FFBS` block + `FFpr` 1 0 | byte-identical |
| FabFilter Pro-Q 4 | `FQ4p` | `FabFilter` | byte-identical |
| FabFilter Pro-R 2 | `FR2p` | `FabFilter` | byte-identical |
| FabFilter Saturn 2 | `FS2a` | `FabFilter` | byte-identical |
| FabFilter Timeless 3 | `F3Ts` | `FabFilter` | byte-identical |
| FabFilter Volcano 3 | `FV3l` | `FabFilter` | byte-identical |
| PITCHMAP (Zynaptiq) | `PTMP` | `LengthPrefixed`: u32 LE length + chunk | byte-identical |
| soothe2 (oeksound) | `SthB` | `AsIs` | identical except JUCE's optional `JUCEPrivateData` tail |
| ValhallaDelay | `dLay` | `VstW`: VST3 SDK hand-over (`VstW` + FXB chunk bank) | contains the VST2 chunk; header version fields differ (ignored by JUCE) |
| ValhallaFreqEcho | `FqEh` | `VstW` | same |
| ValhallaPlate | `plat` | `VstW` | same |
| ValhallaRoom | `Ruum` | `VstW` | same |
| ValhallaShimmer | `shmr` | `VstW` | same |
| ValhallaSpaceModulator | `SpMd` | `VstW` | same |
| ValhallaSupermassive | `sMas` | `VstW` | same |
| ValhallaUberMod | `uber` | `VstW` | same |
| ValhallaVintageVerb | `vee3` | `VstW` | same |
| Vital | `Vita` | `VstW` | same |
| Blackhole (Eventide) | `HOLE` | `VstW` | same |

**Bound parameter slots.** Live binds the first parameters of plugins with few parameters on
its own. The VST3 numbers its parameters differently (ValhallaRoom: VST2 0, 1, 2… → VST3 48,
49, 50…). So:

- A slot that nothing uses is unbound during conversion.
- A device is refused when any of its bound slots is automated (an envelope `PointeeId` points
  at it) or MIDI-mapped (`<KeyMidi>`).

**Not supported from this set.** The two formats of these plugins save different data:

| Plugin | Why |
|---|---|
| Spire | The VST3's processor state is different data. The VST2 chunk reappears as the VST3's controller state. |
| DUNE 3 | The formats store different data (`ngiss` vs a length + `ngisrB`). |

The older FabFilter plugins were also unsupported here; `supply 2.als` below covers them.

### From `test-projects/supply 2 Project/supply 2.als` (Live 12.2, Nov 2026)

FabFilter's older generation, as VST2 and VST3 on the same factory preset (Pro-C 2 "De-essing
bM", Pro-L "Film Score - Gentle and Musical bM"…).

What the formats store:

- **VST2** is parameter-based: Live's `<Buffer>` holds a 28-byte program name, then 0…1
  values (f32 LE).
- **VST3** stores `FabF` `u32 2`, name, `u32 0`, `u32 n`, n *plain* values (f32 LE), then
  `u32 1` `u32 1`.

The values map linearly per parameter, frequencies in log₂ Hz. For example, Pro-C 2 Threshold
is 0…1 → -60…0 dB, and a frequency is 0…1 → log₂ 20…log₂ 20000.

Together with `supply 1.als` (default settings) that gives two points per parameter. Only the
parameters whose VST2 value differs between the two sets can be solved this way:

| Plugin | Parameters solved |
|---|---|
| Pro-C 2 | 17 of 46 |
| Saturn | 105 of 899 |

Volcano 2, Pro-Q 2 and Pro-G also have a different number of parameters in each format (757
vs 709, 191 vs 190, 35 vs 34). A table therefore isn't feasible.

These use `State::VstWParams` instead: the VST3 SDK's VST2 hand-over (`VstW` + FXB bank of
one `FxCk` program). This is how Cubase migrates VST2 projects.

**Checked (Nov 2026).** The owner opened `test-projects/supply 2 Project/supply 2 (修复).als` in
Live 12.2 and re-saved it. `blacksmith --dump` then compared each converted VST3 with the
VST3 next to it on the same preset:

| Plugin | VST2 ID | Result | Status |
|---|---|---|---|
| FabFilter One | `FF01` | state identical | verified |
| FabFilter Timeless 2 | `FTms` | state identical | verified |
| FabFilter Saturn | `FSat` | state identical | verified |
| FabFilter Pro-Q 2 | `FQ2p` | state identical | verified |
| FabFilter Pro-MB | `FPMb` | state identical | verified |
| FabFilter Pro-L 2 | `FL2p` | state identical | verified |
| FabFilter Pro-C 2 | `FC2p` | state identical | verified |
| FabFilter Micro | `FMic` | state identical | verified |
| FabFilter Pro-R | `FPRr` | all 85 values equal | verified |
| FabFilter Pro-L | `FPLr` | all 19 values equal | verified |
| FabFilter Pro-DS | `FPSr` | all 20 values equal | verified |
| FabFilter Volcano 2 | `FV2l` | 5 of 709 values one step higher (e.g. #555 6 vs 5) | unverified (off by default) |
| FabFilter Pro-G | `FPGr` | value #5 is 4 instead of 3 (VST2 had 0.75) | unverified (off by default) |
| FabFilter Simplon | `FSim` | opens on "Default Setting": its VST3 ignores the hand-over | removed |

For Pro-R, Pro-L and Pro-DS, the only difference is the program name. The VST2 cuts it to 28
bytes ("Ambience for Slap Bass "), and that shortened name carries over.

Volcano 2 and Pro-G go wrong only on switch-type parameters, inside FabFilter's own reading of
the hand-over. Fixing them would mean writing their `FabF` state directly, which needs the value
table (more sets at different settings).

## Adding support

1. Load the plugin as VST2 and as VST3 in one project or set, on the same preset. Change a few
   settings the same way on both.
2. Save it into `test-projects/`.
3. Compare the two states (`blacksmith --dump`) and add the rule. For a Live set that is one row
   in `assets/vst2to3.tsv`, unless the plugin needs a new kind of state conversion.
4. Add a row here. Once the host has re-saved the converted copy and `--dump` shows it equal to
   the plugin's own VST3, mark it verified (here and in the table).

# Blacksmith

Drop a DAW project in, get a copy with its VST2 plugins swapped for their VST3
versions — same sounds, same settings.

- **DAW:** FL Studio (`.flp`)
- **Plugins:** Serum

The original is never touched: the upgraded project is saved next to it as
`<name> (VST3).flp`.

```
blacksmith song.flp other.flp   # no window, prints what it did
```

## How it works

FL stores each wrapped plugin as a list of records: its identity (VST2 FourCC
or VST3 class ID), path, and saved state. Serum's VST3 build loads the exact
same state chunk as its VST2 build, so an upgrade swaps the identity and path
for the VST3 ones and moves the chunk into FL's VST3 state layout. Serum's own
parameters keep the same numbers in both formats, so automation still points
at the right knobs.

If the project already has a VST3 Serum in it, that instance is used as the
template (matching your installed version and paths); otherwise a built-in
one is used.

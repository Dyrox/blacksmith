//! VST2 → VST3 of the same plugin in Live sets: how each plugin's VST3 stores
//! what its VST2 saved. The rules are data, one row per plugin in
//! `assets/vst2to3.tsv`; this module holds the state conversions they name.
//! Every rule comes from a set holding both formats of the plugin (listed in
//! `docs/plugins.md`), and only where the VST3's state is an
//! exact function of the VST2's chunk — except `State::VstWParams`, the VST3
//! SDK's hand-over, which only works if the plugin implements it. Plugins
//! whose formats save different data (Spire, DUNE 3…) need more than one set.
//!
//! FL projects don't use this table yet: FL's generic path hands every chunk
//! over in a `VstW` wrapper (`convert.rs`), and an FL rule also needs the
//! VST3's FL template (`plugins.rs`).

use std::sync::OnceLock;

use crate::convert::{fxb_chunk_bank, fxb_param_bank};
use crate::livedb::fuid_to_com;

const TABLE: &str = include_str!("../assets/vst2to3.tsv");

/// How the VST3 holds the VST2's chunk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    /// The chunk itself (JUCE plugins that save their state raw; JUCE's own
    /// trailing `JUCEPrivateData` block is optional and left out).
    AsIs,
    /// The VST3 SDK's VST2 hand-over: `VstW` header + FXB chunk bank (JUCE
    /// plugins built with VST2 compatibility write this themselves).
    VstW,
    /// `u32` little-endian length, then the chunk.
    LengthPrefixed,
    /// FabFilter's current generation: the chunk's `FFBS` parameter block
    /// (`FFBS`, `u32 1`, `u32 n`, n × f32), then `FFpr` `u32 1` `u32 0`. The
    /// preset name and notes after the block go to the VST3's controller state,
    /// which is left empty.
    FabFilter,
    /// A parameter-based VST2 — Live's `<Buffer>` holds the program name (28
    /// bytes) and the values (f32 LE) — handed over the VST3 SDK's way, as
    /// Cubase migrates VST2 projects: `VstW` + FXB bank of that one program.
    /// FabFilter's older generation: their VST3 stores plain values where the
    /// VST2 has 0…1 ones, and two sets didn't pin down every range
    /// (`docs/plugins.md`), so this relies on the plugin reading the hand-over.
    VstWParams,
}

impl State {
    pub fn convert(self, fourcc: u32, chunk: &[u8]) -> Option<Vec<u8>> {
        match self {
            State::AsIs => Some(chunk.to_vec()),
            State::VstW => Some(fxb_chunk_bank(fourcc, chunk)),
            State::LengthPrefixed => {
                let mut out = u32::try_from(chunk.len()).ok()?.to_le_bytes().to_vec();
                out.extend(chunk);
                Some(out)
            }
            State::FabFilter => {
                if chunk.get(..4)? != b"FFBS" {
                    return None;
                }
                let n = u32::from_le_bytes(chunk.get(8..12)?.try_into().ok()?) as usize;
                let mut out = chunk.get(..12 + 4 * n)?.to_vec();
                out.extend(b"FFpr");
                out.extend(1u32.to_le_bytes());
                out.extend(0u32.to_le_bytes());
                Some(out)
            }
            State::VstWParams => {
                let (name, values) = live_params(chunk)?;
                Some(fxb_param_bank(fourcc, &name, &values))
            }
        }
    }

    /// The `state` column of `assets/vst2to3.tsv`.
    fn parse(s: &str) -> Option<State> {
        Some(match s {
            "as-is" => State::AsIs,
            "vstw" => State::VstW,
            "length-prefixed" => State::LengthPrefixed,
            "fabfilter" => State::FabFilter,
            "vstw-params" => State::VstWParams,
            _ => return None,
        })
    }
}

/// Bytes of the program name at the start of Live's buffer for a
/// parameter-based VST2 (the VST2 SDK's `kVstMaxProgNameLen + 4`).
pub const PROGRAM_NAME_LEN: usize = 28;

/// Live's `<Buffer>` for a parameter-based VST2 (`FxBk`): program name, then
/// the values (f32 LE, 0…1).
pub fn live_params(buffer: &[u8]) -> Option<(String, Vec<f32>)> {
    let (name, values) = (buffer.get(..PROGRAM_NAME_LEN)?, buffer.get(PROGRAM_NAME_LEN..)?);
    if values.len() % 4 != 0 {
        return None;
    }
    let name = String::from_utf8_lossy(name.split(|&b| b == 0).next()?).into_owned();
    Some((name, values.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect()))
}

pub struct Rule {
    /// The VST3's name as Live shows it.
    pub name: &'static str,
    pub vendor: &'static str,
    /// Shown in the app, e.g. what isn't carried over exactly.
    pub note: &'static str,
    /// VST2 unique ID (big-endian integer).
    pub fourcc: u32,
    /// VST3 class ID as FL stores it (COM byte order).
    pub class_id: [u8; 16],
    /// As Live writes the VST3: audio inputs, audio outputs, `DeviceType`.
    pub live: (u32, u32, u32),
    pub state: State,
    /// Opened in Live after converting, with its settings.
    pub verified: bool,
}

/// One row: `vst2  vst3  io  state  status  name  vendor  from  [note]`,
/// tab-separated.
/// Comments and malformed rows yield `None`.
fn parse(line: &'static str) -> Option<Rule> {
    let mut f = line.split('\t');
    let fourcc = u32::from_be_bytes(f.next()?.as_bytes().try_into().ok()?);
    let class_id = fuid_to_com(f.next()?)?;
    // Live's `<Vst3PluginInfo>`: audio inputs, audio outputs, `DeviceType`.
    let live = match f.next()? {
        "fx" => (1, 1, 2),
        "fx-stereo" => (2, 1, 2),
        "synth" => (0, 1, 1),
        _ => return None,
    };
    let state = State::parse(f.next()?)?;
    let verified = match f.next()? {
        "verified" => true,
        "unverified" => false,
        _ => return None,
    };
    let name = f.next()?;
    let vendor = f.next()?;
    let _from = f.next()?;
    let note = f.next().unwrap_or("");
    Some(Rule { name, vendor, note, fourcc, class_id, live, state, verified })
}

fn rows() -> impl Iterator<Item = &'static str> {
    TABLE.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
}

/// Every rule in `assets/vst2to3.tsv`.
pub fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| rows().filter_map(parse).collect())
}

/// The rule for a VST2 of this unique ID, if any.
pub fn find(fourcc: u32) -> Option<&'static Rule> {
    rules().iter().find(|r| r.fourcc == fourcc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    #[test]
    fn every_row_parses() {
        for line in rows() {
            assert!(parse(line).is_some(), "bad row in assets/vst2to3.tsv: {line:?}");
        }
        assert_eq!(rules().len(), 32);
    }

    #[test]
    fn class_ids_in_fl_order() {
        // Vital: FL stores `VTSVtivaital…` (same as the FL-side tests).
        assert_eq!(&find(id(b"Vita")).unwrap().class_id, b"VTSVtivaital\0\0\0\0");
        // Pro-Q 3's real (non-Steinberg) ID, as in known_ids.tsv.
        let real = [0x71, 0xdb, 0xc4, 0x72, 0x4d, 0x7a, 0x9a, 0x45, 0xb9, 0x7e, 0x51, 0x74, 0x5d, 0x84, 0xb3, 0x9d];
        assert_eq!(find(id(b"FQ3p")).unwrap().class_id, real);
        // One rule per VST2.
        let rules = rules();
        for (i, r) in rules.iter().enumerate() {
            assert!(rules[i + 1..].iter().all(|o| o.fourcc != r.fourcc && o.class_id != r.class_id), "{}", r.name);
        }
    }

    #[test]
    fn states_convert() {
        let mut ffbs = b"FFBS".to_vec();
        ffbs.extend(1u32.to_le_bytes());
        ffbs.extend(2u32.to_le_bytes());
        ffbs.extend([1.0f32.to_le_bytes(), 0.5f32.to_le_bytes()].concat());
        let mut chunk = ffbs.clone();
        chunk.extend(b"FQ3p\x03\0\0\0\x0f\0\0\0Default Setting");
        let out = State::FabFilter.convert(id(b"FQ3p"), &chunk).unwrap();
        assert_eq!(out, [ffbs, b"FFpr\x01\0\0\0\0\0\0\0".to_vec()].concat());
        assert_eq!(State::FabFilter.convert(0, b"VC2!"), None);

        assert_eq!(State::LengthPrefixed.convert(0, b"<xml/>").unwrap(), b"\x06\0\0\0<xml/>");
        assert_eq!(State::AsIs.convert(0, b"VC2!").unwrap(), b"VC2!");
        let w = State::VstW.convert(id(b"Ruum"), b"VC2!").unwrap();
        assert_eq!((&w[..4], &w[24..28], &w[176..]), (&b"VstW"[..], &b"FBCh"[..], &b"VC2!"[..]));

        // Live's parameter buffer: 28-byte program name, then f32 LE values.
        let mut buf = b"De-essing bM".to_vec();
        buf.resize(PROGRAM_NAME_LEN, 0);
        buf.extend([0.25f32.to_le_bytes(), 1.0f32.to_le_bytes()].concat());
        let w = State::VstWParams.convert(id(b"FC2p"), &buf).unwrap();
        assert_eq!(w, fxb_param_bank(id(b"FC2p"), "De-essing bM", &[0.25, 1.0]));
        assert_eq!(State::VstWParams.convert(0, &buf[..30]), None);
    }
}

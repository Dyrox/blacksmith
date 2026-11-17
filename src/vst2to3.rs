//! VST2 → VST3 of the same plugin in Live sets: how each plugin's VST3 stores
//! what its VST2 saved. Every rule comes from a set holding both formats of the
//! plugin (listed in `docs/plugins.md`), and only where the VST3's state is an
//! exact function of the VST2's chunk; plugins whose formats save different
//! data (parameter-based VST2s, Spire, DUNE 3…) need more than one set.
//!
//! FL projects don't use this table yet: FL's generic path hands every chunk
//! over in a `VstW` wrapper (`convert.rs`), and an FL rule also needs the
//! VST3's FL template (`plugins.rs`).

use crate::convert::fxb_chunk_bank;

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
        }
    }
}

pub struct Rule {
    /// The VST3's name as Live shows it.
    pub name: &'static str,
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

/// A class ID written as Live's FUID (32 hex digits, big-endian words) → FL's
/// byte order.
const fn fuid(hex: &str) -> [u8; 16] {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("not hex"),
        }
    }
    let h = hex.as_bytes();
    assert!(h.len() == 32);
    let mut b = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        b[i] = digit(h[2 * i]) << 4 | digit(h[2 * i + 1]);
        i += 1;
    }
    [b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]
}

const fn id(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const fn rule(name: &'static str, fourcc: &[u8; 4], fuid_hex: &str, live: (u32, u32, u32), state: State) -> Rule {
    Rule { name, fourcc: id(fourcc), class_id: fuid(fuid_hex), live, state, verified: false }
}

const FX: (u32, u32, u32) = (1, 1, 2);
const FX_STEREO: (u32, u32, u32) = (2, 1, 2);
const SYNTH: (u32, u32, u32) = (0, 1, 1);

/// From `supply 1.als` (Live 12, the owner's Mac, Nov 2026).
pub const RULES: &[Rule] = &[
    rule("Pro-Q 3", b"FQ3p", "72c4db717a4d459ab97e51745d84b39d", FX_STEREO, State::FabFilter),
    rule("Pro-Q 4", b"FQ4p", "ed57bd725c60467ea64dd2f400758b6f", FX_STEREO, State::FabFilter),
    rule("Pro-R 2", b"FR2p", "6070873c802a4b078fc06ab5459154e9", FX, State::FabFilter),
    rule("Saturn 2", b"FS2a", "8d067533d8a0491dbaa36c064c6abbfb", FX_STEREO, State::FabFilter),
    rule("Timeless 3", b"F3Ts", "d2ee67f2c552402d902115931afdae6b", FX_STEREO, State::FabFilter),
    rule("Volcano 3", b"FV3l", "299b276ce27b4f5195968a0f8600e9d1", FX_STEREO, State::FabFilter),
    rule("soothe2", b"SthB", "abcdef019182faeb4f656b5353746842", FX_STEREO, State::AsIs),
    rule("PITCHMAP", b"PTMP", "f9522de65a114ab0aa6d0ada740323ed", FX, State::LengthPrefixed),
    rule("ValhallaDelay", b"dLay", "565354644c617976616c68616c6c6164", FX, State::VstW),
    rule("ValhallaFreqEcho", b"FqEh", "5653544671456876616c68616c6c6166", FX, State::VstW),
    rule("ValhallaPlate", b"plat", "565354706c617476616c68616c6c6170", FX, State::VstW),
    rule("ValhallaRoom", b"Ruum", "5653545275756d76616c68616c6c6172", FX, State::VstW),
    rule("ValhallaShimmer", b"shmr", "56535473686d7276616c68616c6c6173", FX, State::VstW),
    rule("ValhallaSpaceModulator", b"SpMd", "56535453704d6476616c68616c6c6173", FX, State::VstW),
    rule("ValhallaSupermassive", b"sMas", "565354734d617376616c68616c6c6173", FX, State::VstW),
    rule("ValhallaUberMod", b"uber", "5653547562657276616c68616c6c6175", FX, State::VstW),
    rule("ValhallaVintageVerb", b"vee3", "5653547665653376616c68616c6c6176", FX, State::VstW),
    rule("Vital", b"Vita", "56535456697461766974616c00000000", SYNTH, State::VstW),
    rule("Blackhole", b"HOLE", "565354484f4c45626c61636b686f6c65", FX, State::VstW),
];

/// The rule for a VST2 of this unique ID, if any.
pub fn find(fourcc: u32) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.fourcc == fourcc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_ids_in_fl_order() {
        // Vital: FL stores `VTSVtivaital…` (same as the FL-side tests).
        assert_eq!(&find(id(b"Vita")).unwrap().class_id, b"VTSVtivaital\0\0\0\0");
        // Pro-Q 3's real (non-Steinberg) ID, as in known_ids.tsv.
        let real = [0x71, 0xdb, 0xc4, 0x72, 0x4d, 0x7a, 0x9a, 0x45, 0xb9, 0x7e, 0x51, 0x74, 0x5d, 0x84, 0xb3, 0x9d];
        assert_eq!(find(id(b"FQ3p")).unwrap().class_id, real);
        // One rule per VST2.
        for (i, r) in RULES.iter().enumerate() {
            assert!(RULES[i + 1..].iter().all(|o| o.fourcc != r.fourcc && o.class_id != r.class_id), "{}", r.name);
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
    }
}

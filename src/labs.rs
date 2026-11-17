//! Spitfire LABS → Splice INSTRUMENT (LABS was discontinued in Oct 2025; its
//! content moved to INSTRUMENT). Both are JUCE plugins that save the same XML
//! preset through JUCE's binary XML (`VC2!`, `u32` length, text, NUL): LABS as
//! `<Labs>`, INSTRUMENT as `<SpliceINSTRUMENT>`, with the instrument named in
//! `<META family=… name=…>`. Seen in `test-projects/supply 3 Project/supply 3.als`
//! (Soft Piano in LABS VST2, LABS VST3 and INSTRUMENT, `docs/plugins.md`).
//!
//! The LABS VST2's chunk and the LABS VST3's state (in a `VstW` hand-over)
//! hold the same XML, and INSTRUMENT reads a `<Labs>` preset itself: handed
//! over as is, it saves exactly what it saves for the same preset loaded fresh.
//! (Rewriting it the way INSTRUMENT writes it, general controls renumbered,
//! carried LABS's values literally — Soft Piano's dynamics 0.0 where
//! INSTRUMENT's own preset has 1.0 — so that was dropped.)

use crate::convert::{fxb_chunk_bank, vstw_chunk};

/// JUCE's binary XML tag.
const XML_MAGIC: &[u8; 4] = b"VC2!";
/// INSTRUMENT's VST2-style ID inside its `VstW` state.
const INSTRUMENT_ID: u32 = u32::from_be_bytes(*b"Inst");

/// The XML text of a JUCE binary XML chunk.
fn xml(chunk: &[u8]) -> Option<&str> {
    if chunk.get(..4)? != XML_MAGIC {
        return None;
    }
    let n = u32::from_le_bytes(chunk.get(4..8)?.try_into().ok()?) as usize;
    std::str::from_utf8(chunk.get(8..8 + n)?).ok()
}

/// `text` as a JUCE binary XML chunk.
fn chunk(text: &str) -> Vec<u8> {
    let mut out = XML_MAGIC.to_vec();
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(text.as_bytes());
    out.push(0);
    out
}

/// The preset XML in a LABS state: a VST2 chunk, or a VST3 state.
fn labs_xml(state: &[u8]) -> Option<&str> {
    let text = xml(vstw_chunk(state).unwrap_or(state))?;
    text.contains("<Labs>").then_some(text)
}

/// LABS state (VST2 chunk or VST3 state) → INSTRUMENT state: the `<Labs>`
/// preset as it is, in INSTRUMENT's `VstW` hand-over.
pub fn to_instrument(state: &[u8]) -> Option<Vec<u8>> {
    Some(fxb_chunk_bank(INSTRUMENT_ID, &chunk(labs_xml(state)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABS: &str = r#"<?xml version="1.0" encoding="UTF-8"?> <Labs><META family="Soft Piano" name="Soft Piano"/><PARAMS><PARAM id="g_gain" value="1.0" cc="7"/><PARAM id="i_reverb" value="0.0" cc="19"/><PARAM id="i_dynamics" value="0.0" cc="1"/></PARAMS></Labs>"#;

    #[test]
    fn states_wrap() {
        // From the VST2's chunk and from the VST3's hand-over alike.
        let vst2 = chunk(LABS);
        let vst3 = fxb_chunk_bank(u32::from_be_bytes(*b"SfLb"), &vst2);
        for state in [&vst2, &vst3] {
            let out = to_instrument(state).unwrap();
            assert_eq!(&out[32..36], b"Inst");
            assert_eq!(xml(vstw_chunk(&out).unwrap()), Some(LABS));
        }
        // Not a LABS preset.
        assert_eq!(to_instrument(b"VC2!\x04\0\0\0<a/>\0"), None);
    }
}

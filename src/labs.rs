//! Spitfire LABS → Splice INSTRUMENT (LABS was discontinued in Oct 2025; its
//! content moved to INSTRUMENT). Both are JUCE plugins that save the same XML
//! preset through JUCE's binary XML (`VC2!`, `u32` length, text, NUL): LABS as
//! `<Labs>`, INSTRUMENT as `<SpliceINSTRUMENT>`, with the instrument named in
//! `<META family=… name=…>`. Seen in `test-projects/supply 3 Project/supply 3.als`
//! (Soft Piano in LABS VST2, LABS VST3 and INSTRUMENT, `docs/plugins.md`).
//!
//! The LABS VST2's chunk and the LABS VST3's state (in a `VstW` hand-over)
//! hold the same XML. INSTRUMENT names the general controls by number
//! (`PARAMS`) and stores none of LABS's per-instrument `<PARAM>`s.

use crate::convert::{fxb_chunk_bank, vstw_chunk};

/// JUCE's binary XML tag.
const XML_MAGIC: &[u8; 4] = b"VC2!";
/// INSTRUMENT's VST2-style ID inside its `VstW` state.
const INSTRUMENT_ID: u32 = u32::from_be_bytes(*b"Inst");

/// LABS `<PARAM id>` → INSTRUMENT's, paired by their MIDI CC (gain 7, pan 10,
/// expression 11, dynamics 1) and order.
const PARAMS: &[(&str, &str)] = &[
    ("g_gain", "g_general_param_1"),
    ("g_pan", "g_general_param_2"),
    ("g_tune", "g_general_param_3"),
    ("i_expression", "g_general_param_4"),
    ("i_dynamics", "g_general_param_5"),
];

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

/// A LABS preset as INSTRUMENT writes it: the root renamed, the general
/// controls renumbered, LABS's other `<PARAM>`s left out (INSTRUMENT falls
/// back to the preset's values).
fn to_instrument(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text.replace("<Labs>", "<SpliceINSTRUMENT>").replace("</Labs>", "</SpliceINSTRUMENT>");
    while let Some(at) = rest.find("<PARAM id=\"") {
        let end = at + rest[at..].find("/>").map_or(rest.len() - at, |e| e + 2);
        out.push_str(&rest[..at]);
        let elem = &rest[at..end];
        let id = elem["<PARAM id=\"".len()..].split('"').next().unwrap_or("");
        if let Some((_, new)) = PARAMS.iter().find(|(old, _)| *old == id) {
            out.push_str(&elem.replacen(id, new, 1));
        } else {
            // Drop the element and the whitespace before it.
            let trimmed = out.trim_end().len();
            out.truncate(trimmed);
        }
        rest = rest[end..].to_string();
    }
    out.push_str(&rest);
    out
}

/// LABS state → INSTRUMENT state, the preset converted (`to_instrument`).
pub fn converted(state: &[u8]) -> Option<Vec<u8>> {
    Some(fxb_chunk_bank(INSTRUMENT_ID, &chunk(&to_instrument(labs_xml(state)?))))
}

/// LABS state → INSTRUMENT state, the `<Labs>` preset as it is. Test variant A
/// (`supply 3 (A 原样).als`), kept until Live decides between the two.
#[allow(dead_code)]
pub fn as_is(state: &[u8]) -> Option<Vec<u8>> {
    Some(fxb_chunk_bank(INSTRUMENT_ID, &chunk(labs_xml(state)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABS: &str = r#"<?xml version="1.0" encoding="UTF-8"?> <Labs><META family="Soft Piano" name="Soft Piano"/><PARAMS><PARAM id="g_gain" value="1.0" cc="7"/><PARAM id="i_reverb" value="0.0" cc="19"/><PARAM id="i_dynamics" value="0.0" cc="1"/></PARAMS></Labs>"#;

    #[test]
    fn preset_moves() {
        let out = to_instrument(LABS);
        assert_eq!(
            out,
            r#"<?xml version="1.0" encoding="UTF-8"?> <SpliceINSTRUMENT><META family="Soft Piano" name="Soft Piano"/><PARAMS><PARAM id="g_general_param_1" value="1.0" cc="7"/><PARAM id="g_general_param_5" value="0.0" cc="1"/></PARAMS></SpliceINSTRUMENT>"#
        );
    }

    #[test]
    fn states_wrap() {
        // From the VST2's chunk and from the VST3's hand-over alike.
        let vst2 = chunk(LABS);
        let vst3 = fxb_chunk_bank(u32::from_be_bytes(*b"SfLb"), &vst2);
        for state in [&vst2, &vst3] {
            let out = as_is(state).unwrap();
            assert_eq!(&out[32..36], b"Inst");
            assert_eq!(xml(vstw_chunk(&out).unwrap()), Some(LABS));
            assert!(xml(vstw_chunk(&converted(state).unwrap()).unwrap()).unwrap().contains("<SpliceINSTRUMENT>"));
        }
        assert_eq!(as_is(b"VC2!\x03\0\0\0<a/>"), None);
    }
}

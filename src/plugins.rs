//! Plugins with a dedicated, tested VST2 → VST3 rule.
//!
//! Add a plugin by giving its VST2 unique ID, its VST3 class ID (as FL stores
//! it), a built-in template (a VST3 instance's records from a real FL project,
//! with an empty component state), and how its VST2 state becomes VST3 state.

use crate::convert::{STATE, Target, Vst2State, get, plugin_records};
use crate::flp::read_records;

pub struct Rule {
    pub name: &'static str,
    pub vendor: &'static str,
    /// Shown in the app's list of supported plugins.
    pub note: &'static str,
    /// VST2 unique ID as an integer (FL stores it little-endian).
    pub fourcc: u32,
    pub class_id: [u8; 16],
    template: &'static [u8],
    pub state: State,
    /// Older VST3 builds of the plugin with another class ID. FL can't load
    /// them with the current VST3 installed; Blacksmith relinks them.
    pub aliases: &'static [Alias],
}

pub struct Alias {
    pub class_id: [u8; 16],
    /// Its VST3 component state → the current one (`None`: not recognised).
    pub state: fn(&[u8]) -> Option<Vec<u8>>,
}

/// How a VST2 state becomes the VST3 component state.
#[derive(Clone, Copy)]
pub enum State {
    /// From a chunk-based VST2.
    Chunk(fn(&[u8]) -> Vec<u8>),
    /// From a parameter-based VST2: its parameter values in order.
    Params(fn(&[f32]) -> Vec<u8>),
}

impl Rule {
    /// The VST3 component state, if this rule handles this kind of VST2 state.
    pub fn component_state(&self, state: &Vst2State) -> Option<Vec<u8>> {
        match (self.state, state) {
            (State::Chunk(f), Vst2State::Chunk(c)) => Some(f(c)),
            (State::Params(f), Vst2State::Params { values, .. }) => Some(f(values)),
            _ => None,
        }
    }

    pub fn builtin_target(&self) -> Target {
        let records = plugin_records(self.template).expect("built-in template");
        let state = get(&records, STATE).and_then(|s| s.get(4..)).and_then(read_records);
        Target { class_id: self.class_id.to_vec(), path: String::from_utf8_lossy(get(&records, 55).unwrap_or_default()).into_owned(), state }
    }
}

pub const RULES: &[Rule] = &[
    Rule {
        name: "Serum",
        vendor: "Xfer Records",
        note: "Serum 1 · 音色原样迁移，已在 FL Studio 2025 验证",
        fourcc: u32::from_be_bytes(*b"XfsX"),
        class_id: *b"XTSVsfsXerum\0\0\0\0",
        template: include_bytes!("../assets/serum-vst3-template.bin"),
        // Serum's VST3 reads its VST2 chunk unchanged (checked in FL Studio 2025).
        state: State::Chunk(<[u8]>::to_vec),
        aliases: &[],
    },
    Rule {
        name: "OTT",
        vendor: "Xfer Records",
        note: "参数原样迁移，已在 FL Studio 2025 验证",
        fourcc: u32::from_be_bytes(*b"XfTT"),
        class_id: *b"XTSVTfoTtt\0\0\0\0\0\0",
        template: include_bytes!("../assets/ott-vst3-template.bin"),
        state: State::Params(ott_state),
        // An older OTT VST3 (projects from the web, Windows): 'VSE' instead of
        // 'VST' in its class ID, 19 parameters instead of 20.
        aliases: &[Alias { class_id: *b"XESVTfoTtt\0\0\0\0\0\0", state: ott_from_old_vst3 }],
    },
];

/// OTT's VST3 state is its parameters as little-endian f32s: the VST2's 19 in
/// the same order, then one the VST2 doesn't have (0 when FL saves a fresh
/// instance). Seen in FL Studio 2025, Nov 2026.
fn ott_state(values: &[f32]) -> Vec<u8> {
    let mut v = values.to_vec();
    v.resize(20, 0.0);
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The older OTT VST3 stores its 19 parameters the same way.
fn ott_from_old_vst3(component: &[u8]) -> Option<Vec<u8>> {
    if component.is_empty() || component.len() % 4 != 0 || component.len() > 80 {
        return None;
    }
    let values: Vec<f32> = component.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    Some(ott_state(&values))
}

/// The rule and alias for a VST3 instance saved by an older build.
pub fn find_alias(class_id: &[u8]) -> Option<(&'static Rule, &'static Alias)> {
    RULES.iter().find_map(|r| r.aliases.iter().find(|a| a.class_id[..] == *class_id).map(|a| (r, a)))
}

pub fn find(fourcc: u32) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.fourcc == fourcc)
}

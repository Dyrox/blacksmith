//! Plugins with a dedicated, tested VST2 → VST3 rule.
//!
//! Add a plugin by giving its VST2 unique ID, its VST3 class ID (as FL stores
//! it), a built-in template (a VST3 instance's records from a real FL project,
//! with an empty component state), and how its VST2 chunk becomes VST3 state.

use crate::convert::{Target, get, plugin_records, STATE};
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
    /// VST2 chunk → VST3 component state.
    pub state: fn(&[u8]) -> Vec<u8>,
}

impl Rule {
    pub fn builtin_target(&self) -> Target {
        let records = plugin_records(self.template).expect("built-in template");
        let state = get(&records, STATE).and_then(|s| s.get(4..)).and_then(read_records);
        Target { class_id: self.class_id.to_vec(), path: String::from_utf8_lossy(get(&records, 55).unwrap_or_default()).into_owned(), state }
    }
}

pub const RULES: &[Rule] = &[Rule {
    name: "Serum",
    vendor: "Xfer Records",
    note: "Serum 1 · 音色原样迁移，已在 FL Studio 2025 验证",
    fourcc: u32::from_be_bytes(*b"XfsX"),
    class_id: *b"XTSVsfsXerum\0\0\0\0",
    template: include_bytes!("../assets/serum-vst3-template.bin"),
    // Serum's VST3 reads its VST2 chunk unchanged (checked in FL Studio 2025).
    state: <[u8]>::to_vec,
}];

pub fn find(fourcc: u32) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.fourcc == fourcc)
}

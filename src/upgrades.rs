//! 升级插件: moving a plugin to a different, newer product — Kontakt (6) →
//! Kontakt 8, Spitfire LABS → Splice INSTRUMENT, later e.g. Serum → Serum 2.
//! When the maker has discontinued the old plugin (`deprecated`), projects show
//! it as 已停产 even where the new one isn't installed. Unlike an older build's ID
//! (`plugins::Alias`, the same plugin), the target is another plugin with its
//! own ID, name and parameters, so these are always the user's choice.
//!
//! Each upgrade names both class IDs (as FL stores them), the target's built-in
//! FL template (its records with an empty component state, from a project with
//! both plugins loaded) and how the old component state becomes the new one.
//! Add one by asking for a project with both plugins holding the same sound.

use crate::check::{Format, Plugin, Status};
use crate::convert::Target;
use crate::labs;
use crate::library::Library;
use crate::plugins::template_target;

/// What an upgrade starts from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    /// A VST3, by class ID (as FL stores it).
    Vst3([u8; 16]),
    /// A VST2, by unique ID (big-endian integer); its chunk is the old state.
    Vst2(u32),
}

pub struct Upgrade {
    pub from_name: &'static str,
    pub to_name: &'static str,
    pub vendor: &'static str,
    /// Shown in the app's list.
    pub note: &'static str,
    /// Checked in a host: the new plugin opens the old state with its sound.
    pub verified: bool,
    /// The old plugin is discontinued: 已停产.
    pub deprecated: bool,
    pub from: Source,
    pub to: [u8; 16],
    /// FL template of the new plugin; without one the upgrade is Live only.
    template: Option<&'static [u8]>,
    /// The new plugin as Live writes it in a `<Vst3PluginInfo>`: audio inputs,
    /// audio outputs, `DeviceType` (needed to turn a VST2 into it in a set).
    pub live: (u32, u32, u32),
    /// Old component state → new (`None`: not recognised).
    pub state: fn(&[u8]) -> Option<Vec<u8>>,
}

impl Upgrade {
    /// The new plugin's FL records (`None`: no FL template yet).
    pub fn builtin_target(&self) -> Option<Target> {
        self.template.map(|t| template_target(&self.to, t))
    }

    /// Whether the host here has the new plugin — by ID, or by file when its
    /// database lists it without one (FL and Kontakt 8).
    pub fn installed_in(&self, lib: &Library) -> bool {
        lib.is_installed(&self.to)
            || self.builtin_target().is_some_and(|t| lib.is_unscanned_installed(&t.path, self.to_name, self.vendor))
    }
}

/// The new plugin reads the old one's state as it is.
fn as_is(state: &[u8]) -> Option<Vec<u8>> {
    Some(state.to_vec())
}

const KONTAKT_8: [u8; 16] = *b"NTSVKik8ontakt 8";
const KONTAKT_8_TEMPLATE: &[u8] = include_bytes!("../assets/kontakt8-vst3-template.bin");
/// Splice INSTRUMENT (FUID `56535449-6e73-7473-706c69636520696e`).
const INSTRUMENT: [u8; 16] = *b"ITSVsnstplice in";
const LABS_NOTE: &str = "Spitfire LABS 已停产（2025 年 10 月起不再更新），音色搬到了 Splice INSTRUMENT：同一个预设交给 INSTRUMENT";

pub const UPGRADES: &[Upgrade] = &[
    Upgrade {
        from_name: "Kontakt（VST3）",
        to_name: "Kontakt 8",
        vendor: "Native Instruments",
        note: "Kontakt 6 → Kontakt 8：存档原样交给 Kontakt 8，乐器和音色不变，已在 FL Studio 2025 和 Ableton Live 12 验证",
        // Checked by the owner (Nov 2026): FL Studio 2025 and a Live set — same instrument and sound.
        verified: true,
        deprecated: true,
        from: Source::Vst3(*b"NTSV$ikDontakt\0\0"),
        to: KONTAKT_8,
        template: Some(KONTAKT_8_TEMPLATE),
        live: (0, 16, 1),
        state: as_is,
    },
    Upgrade {
        from_name: "Kontakt（VST2）",
        to_name: "Kontakt 8",
        vendor: "Native Instruments",
        note: "Kontakt 5/6 的 VST2 → Kontakt 8：VST2 的存档也是 NI 的 hsin 容器，原样交给 Kontakt 8，已在 Ableton Live 12 验证（FL 里还没试）",
        verified: true,
        deprecated: true,
        from: Source::Vst2(u32::from_be_bytes(*b"Ni$D")),
        to: KONTAKT_8,
        template: Some(KONTAKT_8_TEMPLATE),
        live: (0, 16, 1),
        state: as_is,
    },
    Upgrade {
        from_name: "LABS（VST3）",
        to_name: "Splice INSTRUMENT",
        vendor: "Splice",
        note: LABS_NOTE,
        verified: false,
        deprecated: true,
        from: Source::Vst3(*b"STSVLflbabs\0\0\0\0\0"),
        to: INSTRUMENT,
        template: None,
        live: (0, 1, 1),
        state: labs::converted,
    },
    Upgrade {
        from_name: "LABS（VST2）",
        to_name: "Splice INSTRUMENT",
        vendor: "Splice",
        note: LABS_NOTE,
        verified: false,
        deprecated: true,
        from: Source::Vst2(u32::from_be_bytes(*b"SfLb")),
        to: INSTRUMENT,
        template: None,
        live: (0, 1, 1),
        state: labs::converted,
    },
];

/// The upgrade for this plugin, if any.
pub fn of(plugin: &Plugin) -> Option<&'static Upgrade> {
    match plugin.format {
        Format::Vst3 => from_vst3(&plugin.id),
        Format::Vst2 => from_vst2(u32::from_be_bytes(plugin.id[..].try_into().ok()?)),
    }
}

/// `status` of an installed plugin its maker discontinued → 已停产.
pub fn deprecated(status: Status, plugin: &Plugin) -> Status {
    match (&status, of(plugin)) {
        (Status::Installed | Status::Vst2 { installed: Some(true) }, Some(u)) if u.deprecated => Status::Deprecated { successor: u.to_name },
        _ => status,
    }
}

/// The upgrade for a VST3 instance of this class ID, if any.
pub fn from_vst3(class_id: &[u8]) -> Option<&'static Upgrade> {
    UPGRADES.iter().find(|u| matches!(u.from, Source::Vst3(id) if id[..] == *class_id))
}

/// The upgrade for a VST2 instance of this unique ID, if any.
pub fn from_vst2(id: u32) -> Option<&'static Upgrade> {
    UPGRADES.iter().find(|u| u.from == Source::Vst2(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_read() {
        for u in UPGRADES.iter().filter(|u| u.template.is_some()) {
            let t = u.builtin_target().unwrap();
            assert_eq!(t.class_id, u.to);
            assert!(t.state.is_some());
        }
        assert_eq!(from_vst3(b"NTSV$ikDontakt\0\0").map(|u| u.to_name), Some("Kontakt 8"));
        assert_eq!(from_vst2(u32::from_be_bytes(*b"Ni$D")).map(|u| u.to_name), Some("Kontakt 8"));
    }
}

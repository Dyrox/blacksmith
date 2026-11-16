//! 升级插件: moving a plugin to a different, newer product — Kontakt (6) →
//! Kontakt 8 now, later e.g. Serum → Serum 2. Unlike an older build's ID
//! (`plugins::Alias`, the same plugin), the target is another plugin with its
//! own ID, name and parameters, so these are always the user's choice.
//!
//! Each upgrade names both class IDs (as FL stores them), the target's built-in
//! FL template (its records with an empty component state, from a project with
//! both plugins loaded) and how the old component state becomes the new one.
//! Add one by asking for a project with both plugins holding the same sound.

use crate::convert::Target;
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
    pub from: Source,
    pub to: [u8; 16],
    template: &'static [u8],
    /// The new plugin as Live writes it in a `<Vst3PluginInfo>`: audio inputs,
    /// audio outputs, `DeviceType` (needed to turn a VST2 into it in a set).
    pub live: (u32, u32, u32),
    /// Old component state → new (`None`: not recognised).
    pub state: fn(&[u8]) -> Option<Vec<u8>>,
}

impl Upgrade {
    pub fn builtin_target(&self) -> Target {
        template_target(&self.to, self.template)
    }

    /// Whether the host here has the new plugin — by ID, or by file when its
    /// database lists it without one (FL and Kontakt 8).
    pub fn installed_in(&self, lib: &Library) -> bool {
        lib.is_installed(&self.to) || lib.is_unscanned_installed(&self.builtin_target().path, self.to_name, self.vendor)
    }
}

/// The new plugin reads the old one's state as it is.
fn as_is(state: &[u8]) -> Option<Vec<u8>> {
    Some(state.to_vec())
}

const KONTAKT_8: [u8; 16] = *b"NTSVKik8ontakt 8";
const KONTAKT_8_TEMPLATE: &[u8] = include_bytes!("../assets/kontakt8-vst3-template.bin");

pub const UPGRADES: &[Upgrade] = &[
    Upgrade {
        from_name: "Kontakt（VST3）",
        to_name: "Kontakt 8",
        vendor: "Native Instruments",
        note: "Kontakt 6 → Kontakt 8：存档原样交给 Kontakt 8，乐器和音色不变，已在 FL Studio 2025 和 Ableton Live 12 验证",
        // Checked by the owner (Nov 2026): FL Studio 2025 and a Live set — same instrument and sound.
        verified: true,
        from: Source::Vst3(*b"NTSV$ikDontakt\0\0"),
        to: KONTAKT_8,
        template: KONTAKT_8_TEMPLATE,
        live: (0, 16, 1),
        state: as_is,
    },
    Upgrade {
        from_name: "Kontakt（VST2）",
        to_name: "Kontakt 8",
        vendor: "Native Instruments",
        note: "Kontakt 5/6 的 VST2 → Kontakt 8：VST2 的存档也是 NI 的 hsin 容器，原样交给 Kontakt 8，已在 Ableton Live 12 验证（FL 里还没试）",
        verified: true,
        from: Source::Vst2(u32::from_be_bytes(*b"Ni$D")),
        to: KONTAKT_8,
        template: KONTAKT_8_TEMPLATE,
        live: (0, 16, 1),
        state: as_is,
    },
];

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
        for u in UPGRADES {
            let t = u.builtin_target();
            assert_eq!(t.class_id, u.to);
            assert!(t.state.is_some());
        }
        assert_eq!(from_vst3(b"NTSV$ikDontakt\0\0").map(|u| u.to_name), Some("Kontakt 8"));
        assert_eq!(from_vst2(u32::from_be_bytes(*b"Ni$D")).map(|u| u.to_name), Some("Kontakt 8"));
    }
}

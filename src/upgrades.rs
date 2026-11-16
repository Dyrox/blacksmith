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

pub struct Upgrade {
    pub from_name: &'static str,
    pub to_name: &'static str,
    pub vendor: &'static str,
    /// Shown in the app's list.
    pub note: &'static str,
    /// Checked in a host: the new plugin opens the old state with its sound.
    pub verified: bool,
    pub from: [u8; 16],
    pub to: [u8; 16],
    template: &'static [u8],
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

pub const UPGRADES: &[Upgrade] = &[Upgrade {
    from_name: "Kontakt",
    to_name: "Kontakt 8",
    vendor: "Native Instruments",
    note: "Kontakt 6（VST3）→ Kontakt 8：存档原样交给 Kontakt 8，乐器和音色不变，已在 FL Studio 2025 验证",
    // Checked by the owner in FL Studio 2025 (Nov 2026): opens, same instrument and sound.
    verified: true,
    from: *b"NTSV$ikDontakt\0\0",
    to: *b"NTSVKik8ontakt 8",
    template: include_bytes!("../assets/kontakt8-vst3-template.bin"),
    state: as_is,
}];

/// The upgrade for an instance of this class ID, if any.
pub fn from(class_id: &[u8]) -> Option<&'static Upgrade> {
    UPGRADES.iter().find(|u| u.from[..] == *class_id)
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
        assert_eq!(from(b"NTSV$ikDontakt\0\0").map(|u| u.to_name), Some("Kontakt 8"));
    }
}

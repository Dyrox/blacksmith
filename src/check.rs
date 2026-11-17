//! 检查工程, the part every project format shares: a plugin as a project
//! names it, its status on this computer, what can be done, and the user's
//! choices. Each format reads and fixes its own files: `fl.rs` (FL Studio
//! `.flp`), `als.rs` (Ableton Live `.als`); `document.rs` picks one.
//!
//! Status follows how hosts find a plugin: by format and ID, checked against
//! the host's plugin library (`library.rs`). "Installed" means the host finds
//! the plugin; whether a converted state loads is only verified for official
//! rules (`plugins.rs`).

use std::collections::HashMap;

use crate::convert::How;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Format {
    Vst2,
    Vst3,
}

/// A plugin as a project refers to it: what the host looks it up by, and its name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Plugin {
    pub format: Format,
    /// VST2: the unique ID (4 bytes, big-endian); VST3: the class ID in FL's (COM) byte order.
    pub id: Vec<u8>,
    pub name: String,
    pub vendor: String,
}


#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// The host finds it here.
    Installed,
    /// A VST3 whose ID FL doesn't know, but a plugin of that name and vendor is installed.
    Mismatch,
    /// Not installed, nothing of that name either.
    Missing,
    /// A VST2; whether it's installed here (`None`: unknown, no local database).
    Vst2 { installed: Option<bool> },
    /// Installed, but its binary has no arm64: on Apple Silicon the host only
    /// loads it when opened with Rosetta (`arch.rs`).
    Rosetta,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// VST2 → VST3. `target`: the VST3's name; `installed`: FL here has it.
    Upgrade { how: How, target: String, installed: bool },
    /// An older VST3 build's ID → the current one, by an official rule.
    Relink { rule: &'static str },
    /// To the installed VST3 of the same name and vendor (class ID only).
    Rename { target: String },
    /// 升级插件: to a different, newer product (`upgrades.rs`); always opt-in.
    Migrate { to: &'static str, verified: bool },
}

impl Fix {
    /// Whether it's applied unless the user unticks it: verified fixes that end
    /// with a plugin the host can find. Unverified ones stay off: a plugin
    /// handed state it can't read may crash the host (Ample Guitar SC 3.7 took
    /// Live down on an older build's state, Nov 2026).
    pub fn default_on(&self) -> bool {
        match self {
            Fix::Upgrade { installed, .. } => *installed && self.verified(),
            Fix::Relink { .. } => true,
            Fix::Rename { .. } | Fix::Migrate { .. } => false,
        }
    }
    /// Checked in FL: official rules only.
    pub fn verified(&self) -> bool {
        matches!(self, Fix::Upgrade { how: How::Known(_), .. } | Fix::Relink { .. } | Fix::Migrate { verified: true, .. })
    }
}

/// One plugin in a project.
#[derive(Debug, Clone)]
pub struct Finding {
    pub plugin: Plugin,
    pub status: Status,
    pub fix: Option<Fix>,
    pub instances: usize,
}

/// What the user decided for a plugin; plugins without one get their fix if
/// `Fix::default_on`. `reset`: drop the plugin's saved state so it opens with
/// its defaults — settings are lost, but it can't choke on state it can't read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Choice {
    Off,
    /// The suggested fix.
    On { reset: bool },
    /// Point it at this installed VST3 (class ID) instead.
    Pick { id: [u8; 16], reset: bool },
}

impl Choice {
    pub fn reset(self) -> bool {
        matches!(self, Choice::On { reset: true } | Choice::Pick { reset: true, .. })
    }
    pub fn with_reset(self, reset: bool) -> Self {
        match self {
            Choice::Off => Choice::Off,
            Choice::On { .. } => Choice::On { reset },
            Choice::Pick { id, .. } => Choice::Pick { id, reset },
        }
    }
}

pub type Choices = HashMap<Plugin, Choice>;

/// What to do to a plugin's instances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    /// `None`: the suggested fix; `Some`: this installed VST3.
    pub pick: Option<[u8; 16]>,
    pub reset: bool,
}

/// The decision for a plugin with this suggested fix under the user's choice.
pub fn decide(fix: Option<&Fix>, choice: Option<Choice>) -> Option<Decision> {
    match choice {
        Some(Choice::Off) => None,
        Some(Choice::Pick { id, reset }) => Some(Decision { pick: Some(id), reset }),
        Some(Choice::On { reset }) => fix.map(|_| Decision { pick: None, reset }),
        None => fix.filter(|f| f.default_on()).map(|_| Decision { pick: None, reset: false }),
    }
}

/// How many instances `apply` would change.
pub fn pending(findings: &[Finding], choices: &Choices) -> usize {
    findings
        .iter()
        .filter(|f| decide(f.fix.as_ref(), choices.get(&f.plugin).copied()).is_some())
        .map(|f| f.instances)
        .sum()
}

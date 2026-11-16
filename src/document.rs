//! A project file of either host — FL Studio `.flp` or Ableton Live `.als` —
//! and what 检查工程 does with it, so the app and the CLI don't care which.

use std::path::Path;

use crate::check::{Choices, Finding};
use crate::library::{self, Host};
use crate::{als, fl, flp};

pub enum Document {
    Fl(flp::Project),
    Live(als::Set),
}

/// File extensions 检查工程 opens.
pub const EXTENSIONS: [&str; 2] = ["flp", "als"];

pub fn is_document(path: &Path) -> bool {
    path.extension().is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

impl Document {
    /// Reads a project; FL projects must read back byte for byte.
    pub fn read(path: &Path) -> Result<Self, String> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !EXTENSIONS.contains(&ext.as_str()) {
            return Err("不是 .flp 或 .als 工程".into());
        }
        let bytes = std::fs::read(path).map_err(|e| format!("读取失败：{e}"))?;
        if ext == "als" {
            return als::Set::parse(&bytes).map(Document::Live);
        }
        match flp::Project::parse(&bytes) {
            Ok(p) if p.to_bytes() == bytes => Ok(Document::Fl(p)),
            Ok(_) => Err("无法无损读取这个工程".into()),
            Err(e) => Err(format!("无法读取：{e}")),
        }
    }

    pub fn host(&self) -> Host {
        match self {
            Document::Fl(_) => Host::Fl,
            Document::Live(_) => Host::Live,
        }
    }

    /// Every plugin the project uses, against its host's library.
    pub fn analyse(&self) -> Vec<Finding> {
        let lib = library::get(self.host());
        match self {
            Document::Fl(p) => fl::analyse(lib, p),
            Document::Live(s) => als::analyse(lib, s),
        }
    }

    /// Applies the choices; returns how many instances changed.
    pub fn apply(&mut self, choices: &Choices) -> usize {
        let lib = library::get(self.host());
        match self {
            Document::Fl(p) => fl::apply(lib, p, choices),
            Document::Live(s) => als::apply(lib, s, choices),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Document::Fl(p) => p.to_bytes(),
            Document::Live(s) => s.to_bytes(),
        }
    }
}

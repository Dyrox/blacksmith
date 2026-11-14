//! VST3 identity for a VST2 plugin, worked out from the project alone.
//!
//! Installs are often messy (stray VST2/VST3 copies, mismatched versions), so
//! nothing on this machine is scanned. The converter uses, in order: FL's own
//! record of the VST3 class ID, a VST3 instance of the same plugin in the
//! project, and finally Steinberg's rule for a VST3 that replaces a VST2
//! (VST3 SDK, `convertVST2UID_To_FUID`): class ID =
//! `"VST"` + VST2 unique ID (big-endian) + first 9 bytes of the lower-case name.

/// FUID byte order → the COM/GUID byte order FL stores.
fn to_com(f: [u8; 16]) -> Vec<u8> {
    let mut c = f.to_vec();
    c[..4].reverse();
    c[4..6].reverse();
    c[6..8].reverse();
    c
}

fn fuid_from_vst2(fourcc: u32, name: &str) -> [u8; 16] {
    let mut f = [0u8; 16];
    f[..3].copy_from_slice(b"VST");
    f[3..7].copy_from_slice(&fourcc.to_be_bytes());
    for (i, b) in name.to_ascii_lowercase().bytes().take(9).enumerate() {
        f[7 + i] = b;
    }
    f
}

/// The class ID (as FL stores it) Steinberg's rule gives this VST2 plugin.
pub fn class_id_from_vst2(fourcc: u32, name: &str) -> Vec<u8> {
    to_com(fuid_from_vst2(fourcc, name))
}

/// The standard VST3 location for a plugin, on the same OS the VST2 path came
/// from. FL falls back to its plugin database when a path doesn't exist.
pub fn default_path(name: &str, vst2_path: &str) -> String {
    let file = format!("{}.vst3", name.trim());
    if vst2_path.contains(":\\") || vst2_path.starts_with("\\\\") {
        format!("C:\\Program Files\\Common Files\\VST3\\{file}")
    } else {
        format!("/Library/Audio/Plug-Ins/VST3/{file}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| format!("{b:02X}")).collect()
    }

    #[test]
    fn steinberg_rule_matches_serum() {
        // Serum's VST2 ID is 'XfsX'; FL stores its VST3 class ID as below.
        assert_eq!(class_id_from_vst2(u32::from_be_bytes(*b"XfsX"), "Serum"), b"XTSVsfsXerum\0\0\0\0");
    }

    #[test]
    fn steinberg_rule_matches_vital() {
        // FL's own record in a real project: 'VTSVtivaital'.
        assert_eq!(class_id_from_vst2(u32::from_be_bytes(*b"Vita"), "Vital"), b"VTSVtivaital\0\0\0\0");
    }

    #[test]
    fn steinberg_forum_example() {
        // VST2 'lmsy' "superchord" → 56535479736D6C737570657263686F72 (SDK output)
        assert_eq!(hex(&fuid_from_vst2(u32::from_be_bytes(*b"ysml"), "superchord")), "56535479736D6C737570657263686F72");
    }

    #[test]
    fn default_paths() {
        assert_eq!(default_path("Vital", "C:\\VstPlugins\\Vital.dll"), "C:\\Program Files\\Common Files\\VST3\\Vital.vst3");
        assert_eq!(default_path("Vital", "/Library/Audio/Plug-Ins/VST/Vital.vst"), "/Library/Audio/Plug-Ins/VST3/Vital.vst3");
    }
}

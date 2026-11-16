//! Ableton Live sets (`.als`) for 检查工程: gzip-compressed XML, checked like
//! FL projects (`check.rs` types) against Live's plugin library (`library.rs`,
//! from Live's own database, `livedb.rs`).
//!
//! A plugin is a `<PluginDevice>` whose `<PluginDesc>` holds one of
//! - `<VstPluginInfo>`: `<UniqueId Value>` (VST2 unique ID, decimal), `<PlugName>`, `<Path>`;
//!   state in `<Preset><VstPreset><Buffer>`.
//! - `<Vst3PluginInfo>`: `<Name>`, `<Uid><Fields.0..3>` — the FUID as four
//!   big-endian 32-bit words written as signed decimals (`ABCDEF01` → -1412567295).
//!   `<Preset><Vst3Preset>` repeats the `<Uid>`; state in `<ProcessorState>`.
//! Sets don't store plugin vendors.
//!
//! Fixes change only the ID digits inside the plugin's block; every other
//! byte of the XML stays as Live wrote it (the gzip wrapper is rewritten).
//! Phase 1: VST3 IDs only; VST2 → VST3 needs per-plugin state conversion.

use std::io::{Read, Write};

use crate::check::{self, Choice, Choices, Finding, Fix, Format, Plugin, Status};
use crate::library::{Entry, Library};
use crate::upgrades;

pub struct Set {
    pub xml: String,
}

/// One plugin instance: where its info block is, and what it names.
struct Instance {
    plugin: Plugin,
    /// Byte range of the `<…PluginInfo …>…</…PluginInfo>` block.
    start: usize,
    end: usize,
    /// Byte range of its `<PluginDevice …>…</PluginDevice>`.
    device: Option<(usize, usize)>,
}

impl Set {
    pub fn parse(bytes: &[u8]) -> Result<Set, String> {
        let mut xml = String::new();
        flate2::read::GzDecoder::new(bytes).read_to_string(&mut xml).map_err(|_| "不是 Ableton Live Set（无法解压）".to_string())?;
        if !xml.starts_with("<?xml") || !xml.contains("<Ableton") {
            return Err("不是 Ableton Live Set".into());
        }
        Ok(Set { xml })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(self.xml.as_bytes()).expect("writing to memory");
        gz.finish().expect("writing to memory")
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The `Value` of the first `<tag Value="…"` in `s`, with its offset.
fn value_of<'a>(s: &'a str, tag: &str) -> Option<(usize, &'a str)> {
    let open = format!("<{tag} Value=\"");
    let at = s.find(&open)? + open.len();
    let len = s[at..].find('"')?;
    Some((at, &s[at..at + len]))
}

/// The four `Fields` of the first `<Uid>` in `s` → the class ID as FL stores it.
fn uid(s: &str) -> Option<[u8; 16]> {
    let u = &s[s.find("<Uid>")?..];
    let mut fuid = [0u8; 16];
    for i in 0..4 {
        let (_, v) = value_of(u, &format!("Fields.{i}"))?;
        let word = v.parse::<i64>().ok()? as u32;
        fuid[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    Some(fuid_to_com(fuid))
}

fn fuid_to_com(mut b: [u8; 16]) -> [u8; 16] {
    b[..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    b
}

/// Every VST2 and VST3 instance in the set, in order.
fn instances(xml: &str) -> Vec<Instance> {
    let mut out = Vec::new();
    for (tag, format) in [("VstPluginInfo", Format::Vst2), ("Vst3PluginInfo", Format::Vst3)] {
        let (open, close) = (format!("<{tag} "), format!("</{tag}>"));
        let mut from = 0;
        while let Some(i) = xml[from..].find(&open) {
            let start = from + i;
            let Some(len) = xml[start..].find(&close) else { break };
            let end = start + len + close.len();
            from = end;
            let block = &xml[start..end];
            // The info's own fields sit outside its <Preset>.
            let outside = match (block.find("<Preset>"), block.find("</Preset>")) {
                (Some(a), Some(b)) => format!("{}{}", &block[..a], &block[b..]),
                _ => block.to_string(),
            };
            let plugin = match format {
                Format::Vst2 => {
                    let Some(id) = value_of(&outside, "UniqueId").and_then(|(_, v)| v.parse::<i64>().ok()) else { continue };
                    let name = value_of(&outside, "PlugName").map(|(_, v)| unescape(v)).unwrap_or_default();
                    Plugin { format, id: (id as u32).to_be_bytes().to_vec(), name, vendor: String::new() }
                }
                Format::Vst3 => {
                    let Some(id) = uid(&outside) else { continue };
                    let name = value_of(&outside, "Name").map(|(_, v)| unescape(v)).unwrap_or_default();
                    Plugin { format, id: id.to_vec(), name, vendor: String::new() }
                }
            };
            let device = xml[..start].rfind("<PluginDevice ").and_then(|ds| {
                let close = "</PluginDevice>";
                let de = end + xml[end..].find(close)? + close.len();
                (!xml[ds..start].contains(close)).then_some((ds, de))
            });
            out.push(Instance { plugin, start, end, device });
        }
    }
    out.sort_by_key(|i| i.start);
    out
}

fn assess(lib: &Library, plugin: &Plugin) -> (Status, Option<Fix>) {
    match plugin.format {
        Format::Vst2 => {
            let id = u32::from_be_bytes(plugin.id[..].try_into().unwrap());
            let fix = upgrades::from_vst2(id).filter(|u| u.installed_in(lib)).map(|u| Fix::Migrate { to: u.to_name, verified: u.verified });
            (Status::Vst2 { installed: lib.vst2_installed(id) }, fix)
        }
        Format::Vst3 => {
            let (status, fix) = if lib.is_installed(&plugin.id) {
                (Status::Installed, None)
            } else {
                match target_by_name(lib, plugin) {
                    Some(k) => (Status::Mismatch, Some(Fix::Rename { target: k.name.to_string() })),
                    None => (Status::Missing, None),
                }
            };
            // A newer product that's installed here beats a rename.
            match upgrades::from_vst3(&plugin.id).filter(|u| u.installed_in(lib)) {
                Some(u) => (status, Some(Fix::Migrate { to: u.to_name, verified: u.verified })),
                None => (status, fix),
            }
        }
    }
}

/// The one installed VST3 of this name (sets don't store vendors).
fn target_by_name<'a>(lib: &'a Library, plugin: &Plugin) -> Option<&'a Entry> {
    match lib.matching(&plugin.name, "")[..] {
        [k] if k.class_id[..] != plugin.id[..] => Some(k),
        _ => None,
    }
}

/// Every plugin the set uses, once per plugin with its instance count.
pub fn analyse(lib: &Library, set: &Set) -> Vec<Finding> {
    let all = instances(&set.xml);
    let mut out: Vec<Finding> = Vec::new();
    for inst in &all {
        if let Some(f) = out.iter_mut().find(|f| f.plugin == inst.plugin) {
            f.instances += 1;
            continue;
        }
        let (status, fix) = assess(lib, &inst.plugin);
        out.push(Finding { plugin: inst.plugin.clone(), status, fix, instances: 1 });
    }
    // A VST2 is only moved when every instance can be.
    for f in out.iter_mut().filter(|f| f.plugin.format == Format::Vst2) {
        if all.iter().any(|i| i.plugin == f.plugin && !vst2_movable(&set.xml, i)) {
            f.fix = None;
        }
    }
    out
}

/// Whether a VST2 device can become a VST3 one: it has a device around it and
/// a chunk, and none of its parameter slots is bound to a VST2 parameter (the
/// VST3's parameters are numbered differently, so automation and mappings on
/// them would point at the wrong controls).
fn vst2_movable(xml: &str, inst: &Instance) -> bool {
    let Some((ds, de)) = inst.device else { return false };
    let device = &xml[ds..de];
    let block = &xml[inst.start..inst.end];
    block.contains("<Buffer>")
        && device.match_indices("<ParameterId Value=\"").all(|(i, m)| device[i + m.len()..].starts_with("-1\""))
}

/// What an instance becomes under the choices.
struct Change {
    class_id: [u8; 16],
    reset: bool,
    /// 升级插件: the new product's name and how its state converts.
    upgrade: Option<&'static upgrades::Upgrade>,
    /// The instance is a VST2 that becomes the upgrade's VST3.
    from_vst2: bool,
}

fn target(lib: &Library, plugin: &Plugin, choice: Option<Choice>) -> Option<Change> {
    let (_, fix) = assess(lib, plugin);
    let decision = check::decide(fix.as_ref(), choice)?;
    if plugin.format == Format::Vst2 {
        // Only an upgrade moves a VST2 (no picks: its state needs a rule).
        let u = upgrades::from_vst2(u32::from_be_bytes(plugin.id[..].try_into().ok()?))?;
        return (decision.pick.is_none()).then_some(Change { class_id: u.to, reset: decision.reset, upgrade: Some(u), from_vst2: true });
    }
    let (class_id, upgrade) = match (decision.pick, &fix) {
        (Some(id), _) => (id, None),
        (None, Some(Fix::Migrate { .. })) => {
            let u = upgrades::from_vst3(&plugin.id)?;
            (u.to, Some(u))
        }
        (None, _) => (target_by_name(lib, plugin)?.class_id, None),
    };
    Some(Change { class_id, reset: decision.reset, upgrade, from_vst2: false })
}

/// The four `<Fields.N>` words of a class ID (as FL stores it), as Live writes them.
fn uid_words(class_id: [u8; 16]) -> [i32; 4] {
    let fuid = fuid_to_com(class_id); // the swap is its own inverse
    std::array::from_fn(|i| u32::from_be_bytes(fuid[4 * i..4 * i + 4].try_into().unwrap()) as i32)
}

/// A `<VstPluginInfo>` block as the upgrade's `<Vst3PluginInfo>`: window
/// position, on/off and overwrite number carried over, the chunk through the
/// upgrade's conversion as `<ProcessorState>` (kept verbatim, line breaks and
/// all, when the conversion leaves it as is).
fn vst3_info_from_vst2(block: &str, up: &upgrades::Upgrade, reset: bool) -> Option<String> {
    let val = |tag: &str, default: &'static str| value_of(block, tag).map_or(default.to_string(), |(_, v)| v.to_string());
    let id = block.strip_prefix("<VstPluginInfo Id=\"").and_then(|r| r.split('"').next()).unwrap_or("0");
    let preset_id = block.find("<VstPreset Id=\"").and_then(|i| block[i + 15..].split('"').next()).unwrap_or("0");
    let (a, b) = (block.find("<Buffer>")? + "<Buffer>".len(), block.find("</Buffer>")?);
    let text = &block[a..b];
    let hex: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let old: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect::<Option<_>>()?;
    let new = (up.state)(&old)?;
    let state = if reset {
        "<ProcessorState />".to_string()
    } else if new == old {
        format!("<ProcessorState>{text}</ProcessorState>")
    } else {
        format!("<ProcessorState>{}</ProcessorState>", new.iter().map(|b| format!("{b:02X}")).collect::<String>())
    };
    let w = uid_words(up.to);
    let uid = format!(
        "<Uid>\n<Fields.0 Value=\"{}\" />\n<Fields.1 Value=\"{}\" />\n<Fields.2 Value=\"{}\" />\n<Fields.3 Value=\"{}\" />\n</Uid>",
        w[0], w[1], w[2], w[3]
    );
    let (inputs, outputs, device_type) = up.live;
    Some(format!(
        r#"<Vst3PluginInfo Id="{id}">
<WinPosX Value="{x}" />
<WinPosY Value="{y}" />
<NumAudioInputs Value="{inputs}" />
<NumAudioOutputs Value="{outputs}" />
<IsPlaceholderDevice Value="false" />
<Preset>
<Vst3Preset Id="{preset_id}">
<OverwriteProtectionNumber Value="{opn}" />
<MpeEnabled Value="0" />
<MpeSettings>
<ZoneType Value="0" />
<FirstNoteChannel Value="1" />
<LastNoteChannel Value="15" />
</MpeSettings>
<ParameterSettings />
<IsOn Value="{on}" />
<PowerMacroControlIndex Value="-1" />
<PowerMacroMappingRange>
<Min Value="64" />
<Max Value="127" />
</PowerMacroMappingRange>
<IsFolded Value="false" />
<StoredAllParameters Value="true" />
<DeviceLomId Value="0" />
<DeviceViewLomId Value="0" />
<IsOnLomId Value="0" />
<ParametersListWrapperLomId Value="0" />
{uid}
<DeviceType Value="{device_type}" />
{state}
<ControllerState />
<Name Value="" />
<PresetRef />
</Vst3Preset>
</Preset>
<Name Value="{name}" />
{uid}
<DeviceType Value="{device_type}" />
</Vst3PluginInfo>"#,
        x = val("WinPosX", "0"),
        y = val("WinPosY", "0"),
        opn = val("OverwriteProtectionNumber", "3074"),
        on = val("IsOn", "true"),
        name = up.to_name,
    ))
}

/// A device's `<BranchDeviceId>` (the browser's link to the plugin) pointed
/// at the new VST3: `device:vst:<kind>:…` → `device:vst3:<kind>:<FUID>`.
fn with_branch_id(device: &str, class_id: [u8; 16]) -> String {
    let tag = "<BranchDeviceId Value=\"device:vst:";
    let Some(i) = device.find(tag) else { return device.to_string() };
    let at = i + tag.len();
    let Some(len) = device[at..].find('"') else { return device.to_string() };
    let kind = device[at..at + len].split(':').next().unwrap_or("instr");
    let f = fuid_to_com(class_id);
    let hex: String = f.iter().map(|b| format!("{b:02x}")).collect();
    let fuid = format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]);
    let mut out = device.to_string();
    out.replace_range(i..at + len, &format!("<BranchDeviceId Value=\"device:vst3:{kind}:{fuid}"));
    out
}

/// `block` (a VST3 info) under another product: its info `<Name>` and its
/// `<ProcessorState>` (hex) through the upgrade's conversion.
fn upgraded(block: &str, up: &upgrades::Upgrade) -> Option<String> {
    let mut out = block.to_string();
    let (open, close) = ("<ProcessorState>", "</ProcessorState>");
    if let (Some(a), Some(b)) = (out.find(open), out.find(close)) {
        let hex: String = out[a + open.len()..b].chars().filter(|c| c.is_ascii_hexdigit()).collect();
        let bytes: Option<Vec<u8>> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect();
        let new = (up.state)(&bytes?)?;
        if new.len() * 2 != hex.len() || new.iter().zip(hex.as_bytes().chunks(2)).any(|(n, h)| format!("{n:02X}").as_bytes() != h) {
            let text: String = new.iter().map(|b| format!("{b:02X}")).collect();
            out.replace_range(a + open.len()..b, &text);
        }
    }
    // The info's own name sits after its <Preset>.
    let after = out.find("</Preset>").map_or(0, |i| i + "</Preset>".len());
    let tag = "<Name Value=\"";
    if let Some(i) = out[after..].find(tag) {
        let at = after + i + tag.len();
        if let Some(len) = out[at..].find('"') {
            out.replace_range(at..at + len, up.to_name);
        }
    }
    Some(out)
}

/// `block` with its saved VST3 state emptied: Live then opens the plugin with
/// its defaults.
fn without_state(block: &str) -> String {
    let mut out = block.to_string();
    for tag in ["ProcessorState", "ControllerState"] {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        if let (Some(a), Some(b)) = (out.find(&open), out.find(&close)) {
            out.replace_range(a..b + close.len(), &format!("<{tag} />"));
        }
    }
    out
}

/// `block` with every `<Fields.N Value="…"` (both `<Uid>`s) set to `class_id`.
fn with_uid(block: &str, class_id: [u8; 16]) -> String {
    let fuid = fuid_to_com(class_id); // the swap is its own inverse
    let mut out = block.to_string();
    for i in 0..4 {
        let word = u32::from_be_bytes(fuid[4 * i..4 * i + 4].try_into().unwrap()) as i32;
        let open = format!("<Fields.{i} Value=\"");
        let mut from = 0;
        while let Some(at) = out[from..].find(&open) {
            let at = from + at + open.len();
            let Some(len) = out[at..].find('"') else { break };
            out.replace_range(at..at + len, &word.to_string());
            from = at;
        }
    }
    out
}

/// Applies the choices (and the default fixes); returns how many instances changed.
pub fn apply(lib: &Library, set: &mut Set, choices: &Choices) -> usize {
    let mut changed = 0;
    // Back to front, so earlier offsets stay valid.
    for inst in instances(&set.xml).into_iter().rev() {
        if let Some(change) = target(lib, &inst.plugin, choices.get(&inst.plugin).copied()) {
            if change.from_vst2 {
                let (Some(up), Some((ds, de))) = (change.upgrade, inst.device) else { continue };
                if !vst2_movable(&set.xml, &inst) {
                    continue;
                }
                let Some(info) = vst3_info_from_vst2(&set.xml[inst.start..inst.end], up, change.reset) else { continue };
                let device = format!("{}{}{}", &set.xml[ds..inst.start], info, &set.xml[inst.end..de]);
                set.xml.replace_range(ds..de, &with_branch_id(&device, up.to));
                changed += 1;
                continue;
            }
            let block = with_uid(&set.xml[inst.start..inst.end], change.class_id);
            let block = match change.upgrade {
                Some(up) => match upgraded(&block, up) {
                    Some(b) => b,
                    None => continue,
                },
                None => block,
            };
            let block = if change.reset { without_state(&block) } else { block };
            set.xml.replace_range(inst.start..inst.end, &block);
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Host, get as library};

    const SET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton MajorVersion="5" Creator="Ableton Live 12.2.1">
	<PluginDevice Id="1"><PluginDesc>
		<Vst3PluginInfo Id="0">
			<Preset><Vst3Preset Id="3">
				<Uid><Fields.0 Value="1448301654" /><Fields.1 Value="1769234806" /><Fields.2 Value="1769234796" /><Fields.3 Value="0" /></Uid>
				<ProcessorState>0A0B</ProcessorState><Name Value="" />
			</Vst3Preset></Preset>
			<Name Value="Vital" />
			<Uid><Fields.0 Value="1448301654" /><Fields.1 Value="1769234806" /><Fields.2 Value="1769234796" /><Fields.3 Value="0" /></Uid>
		</Vst3PluginInfo>
	</PluginDesc></PluginDevice>
	<PluginDevice Id="2"><PluginDesc>
		<VstPluginInfo Id="0">
			<Path Value="/Library/Audio/Plug-Ins/VST/ValhallaShimmer_x64.vst" />
			<PlugName Value="Valhalla &amp; Shimmer" />
			<UniqueId Value="1936223602" />
			<Preset><VstPreset Id="2"><UniqueId Value="1" /><Buffer>00</Buffer></VstPreset></Preset>
		</VstPluginInfo>
	</PluginDesc></PluginDevice>
	<PluginDevice Id="3"><PluginDesc>
		<Vst3PluginInfo Id="0">
			<Name Value="JUCE thing" />
			<Uid><Fields.0 Value="-1412567295" /><Fields.1 Value="-1852769557" /><Fields.2 Value="1" /><Fields.3 Value="2" /></Uid>
		</Vst3PluginInfo>
	</PluginDesc></PluginDevice>
</Ableton>
"#;

    fn set() -> Set {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(SET.as_bytes()).unwrap();
        Set::parse(&gz.finish().unwrap()).unwrap()
    }

    #[test]
    fn finds_vst2_and_vst3_instances() {
        let found = instances(&set().xml);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].plugin.format, Format::Vst3);
        assert_eq!(found[0].plugin.name, "Vital");
        assert_eq!(found[0].plugin.id, b"VTSVtivaital\0\0\0\0");
        assert_eq!(found[1].plugin.format, Format::Vst2);
        assert_eq!(found[1].plugin.name, "Valhalla & Shimmer");
        assert_eq!(found[1].plugin.id, 1936223602u32.to_be_bytes());
        // ABCDEF01 stored as a negative word.
        assert_eq!(found[2].plugin.id[..4], [0x01, 0xef, 0xcd, 0xab]);
    }

    #[test]
    fn picking_changes_only_the_id_digits() {
        let mut s = set();
        let jucey = instances(&s.xml)[2].plugin.clone();
        let mut choices = Choices::new();
        choices.insert(jucey.clone(), Choice::Pick { id: *b"XTSVsfsXerum\0\0\0\0", reset: false });
        assert_eq!(apply(library(Host::Live), &mut s, &choices), 1);
        let after = instances(&s.xml);
        assert_eq!(after[2].plugin.id, b"XTSVsfsXerum\0\0\0\0");
        assert_eq!(after[2].plugin.name, "JUCE thing");
        // Everything else is untouched.
        assert_eq!(&s.xml[..after[2].start], &SET[..after[2].start]);
        assert!(s.xml.contains(r#"<Fields.0 Value="1448301654" />"#));
        assert!(!s.xml.contains("-1412567295"));
        // Round trip through gzip.
        assert_eq!(Set::parse(&s.to_bytes()).unwrap().xml, s.xml);
    }

    #[test]
    fn vst3_uid_in_preset_changes_too() {
        let mut s = set();
        let vital = instances(&s.xml)[0].plugin.clone();
        let mut choices = Choices::new();
        choices.insert(vital, Choice::Pick { id: [7; 16], reset: true });
        apply(library(Host::Live), &mut s, &choices);
        assert_eq!(s.xml.matches("1448301654").count(), 0);
        assert_eq!(instances(&s.xml)[0].plugin.id, [7; 16]);
        // Reset: the state is gone, the rest of the block stays.
        assert!(s.xml.contains("<ProcessorState />") && !s.xml.contains("0A0B"));
        assert!(s.xml.contains("<Name Value=\"Vital\" />"));
    }

    #[test]
    fn upgrade_renames_and_keeps_state() {
        let up = upgrades::from_vst3(b"NTSV$ikDontakt\0\0").unwrap();
        let block = r#"<Vst3PluginInfo Id="0"><Preset><Vst3Preset Id="3"><ProcessorState>0A0B</ProcessorState><Name Value="" /></Vst3Preset></Preset><Name Value="Kontakt" /></Vst3PluginInfo>"#;
        let out = upgraded(block, up).unwrap();
        assert!(out.contains(r#"</Preset><Name Value="Kontakt 8" />"#));
        assert!(out.contains("<ProcessorState>0A0B</ProcessorState>"));
        assert!(out.contains(r#"<Vst3Preset Id="3"><ProcessorState>0A0B</ProcessorState><Name Value="" />"#));
    }

    const KONTAKT_VST2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton MajorVersion="5">
<PluginDevice Id="7"><On /><SourceContext><Value><BranchSourceContext Id="0">
<BranchDeviceId Value="device:vst:instr:1315513412?n=Kontakt" />
</BranchSourceContext></Value></SourceContext>
<PluginDesc>
<VstPluginInfo Id="0">
<WinPosX Value="12" />
<WinPosY Value="34" />
<Path Value="E:/Plugin 64/Kontakt.dll" />
<PlugName Value="Kontakt" />
<UniqueId Value="1315513412" />
<Preset>
<VstPreset Id="5">
<OverwriteProtectionNumber Value="2816" />
<IsOn Value="false" />
<Buffer>
98720E0000000000
6873696E
</Buffer>
<UniqueId Value="1315513412" />
</VstPreset>
</Preset>
</VstPluginInfo>
</PluginDesc>
<ParameterList><PluginFloatParameter Id="0"><ParameterId Value="-1" /></PluginFloatParameter></ParameterList>
</PluginDevice>
</Ableton>
"#;

    #[test]
    fn kontakt_vst2_device_becomes_kontakt_8() {
        let up = upgrades::from_vst2(u32::from_be_bytes(*b"Ni$D")).unwrap();
        let found = instances(KONTAKT_VST2);
        assert_eq!(found.len(), 1);
        assert!(vst2_movable(KONTAKT_VST2, &found[0]));
        let inst = &found[0];
        let info = vst3_info_from_vst2(&KONTAKT_VST2[inst.start..inst.end], up, false).unwrap();
        let (ds, de) = inst.device.unwrap();
        let device = format!("{}{}{}", &KONTAKT_VST2[ds..inst.start], info, &KONTAKT_VST2[inst.end..de]);
        let device = with_branch_id(&device, up.to);
        let xml = format!("{}{}{}", &KONTAKT_VST2[..ds], device, &KONTAKT_VST2[de..]);
        // It reads back as Kontakt 8's VST3 with the chunk verbatim.
        let after = instances(&xml);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].plugin.format, Format::Vst3);
        assert_eq!(after[0].plugin.id, up.to);
        assert_eq!(after[0].plugin.name, "Kontakt 8");
        assert!(xml.contains("<ProcessorState>\n98720E0000000000\n6873696E\n</ProcessorState>"));
        assert!(xml.contains(r#"<WinPosX Value="12" />"#) && xml.contains(r#"<IsOn Value="false" />"#));
        assert!(xml.contains(r#"<Vst3Preset Id="5">"#) && xml.contains(r#"<OverwriteProtectionNumber Value="2816" />"#));
        assert!(xml.contains("device:vst3:instr:5653544e-694b-386b-6f6e-74616b742038"));
        assert!(!xml.contains("VstPluginInfo") && !xml.contains("<Buffer>"));
        // A bound parameter slot stops it.
        let bound = KONTAKT_VST2.replace(r#"<ParameterId Value="-1" />"#, r#"<ParameterId Value="3" />"#);
        assert!(!vst2_movable(&bound, &instances(&bound)[0]));
    }

    #[test]
    fn rejects_other_files() {
        assert!(Set::parse(b"FLhd").is_err());
    }
}

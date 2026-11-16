//! VST2 → VST3 upgrades inside an FL Studio project.
//!
//! FL keeps each wrapped plugin in a `PLUGIN_DATA` event: a u32 kind (10 for
//! VST) followed by records (see `flp::read_records`). The ones that differ:
//!
//! | id | VST2                          | VST3                                          |
//! |----|-------------------------------|-----------------------------------------------|
//! | 50 | flags (0)                     | flags (8)                                     |
//! | 51 | FourCC (unique ID, LE)        | —                                             |
//! | 52 | —                             | class ID (16 bytes, COM byte order)           |
//! | 55 | path to the .dll              | path to the binary inside the .vst3 bundle    |
//! | 57 | 0x960 on every VST2 seen (meaning unknown) | —                                |
//! | 53 | 21-byte header + plugin chunk | records: 1 header · 3 component state · 4 parameter ID table |
//!
//! Each VST2 instance is matched in order against:
//! 1. a **known plugin** (`plugins.rs`): exact class ID and a verified state
//!    conversion;
//! 2. the **generic** path: find the VST3 identity without scanning the
//!    machine — a VST3 instance of the same plugin in the project, else the
//!    plugin library (`library.rs`), else FL's own class ID record on the
//!    instance, else Steinberg's naming rule (`vst3.rs`) and hand it the VST2 state the way Steinberg's spec says
//!    hosts must: as an FXB bank (`VST3::tryVst2StateLoad` in the VST3 SDK).

use crate::flp::{Project, Records, read_records, write_records};
use crate::library::{Entry, Library};
use crate::{plugins, vst3};

pub const PLUGIN_DATA: u8 = 213;
/// First u32 of FL's wrapper data: a format version, not a plugin type. Seen:
/// 10 (FL 12–21), 12 (FL 24–25), 8 and 9 in between; flpdiff reports 6–12.
/// Kept as is when converting.
const WRAPPER_VERSIONS: std::ops::RangeInclusive<u32> = 6..=12;
#[cfg(test)]
const KIND_VST: u32 = 10;

#[cfg(test)]
pub const WINDOW: u32 = 2;
pub const FLAGS: u32 = 50;
pub const FOURCC: u32 = 51;
pub const CLASS_ID: u32 = 52;
pub const STATE: u32 = 53;
pub const NAME: u32 = 54;
pub const PATH: u32 = 55;
pub const VENDOR: u32 = 56;
/// Not the parameter count (it is 0x960 on every VST2 seen); dropped on conversion.
pub const VST2_57: u32 = 57;

const VST3_STATE_VERSION: u32 = 1;
pub const VST3_HEADER: u32 = 1;
pub const VST3_COMPONENT: u32 = 3;
pub const VST3_PARAM_IDS: u32 = 4;
const VST2_HEADER: usize = 21;
const VST3_FLAG: u32 = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum How {
    /// A plugin with a dedicated, tested rule.
    Known(&'static str),
    /// Generic conversion: follows the VST3 spec, but not verified for this
    /// plugin. `derived`: the VST3 class ID came from Steinberg's naming rule
    /// rather than from FL or a VST3 instance in the project, so it may not
    /// match what the plugin really uses.
    Generic { derived: bool },
    /// Already a VST3 instance; left as is (by `upgrade`, used in tests).
    #[cfg(test)]
    AlreadyVst3,
    /// 修复: a VST3 saved by an older build under another class ID, moved to
    /// the current one by an official rule (state converted too).
    Relinked(&'static str),
    /// 修复: a VST3 whose class ID isn't in the library, given the library's ID
    /// for the plugin of the same name and vendor. State untouched.
    Reidentified,
    /// 修复: class ID is in the library (or has a rule) — nothing to do.
    Recognized,
    /// 修复: class ID unknown and no library plugin of that name and vendor.
    NotInLibrary,
    Skipped(String),
}

/// What a whole-project pass did; the app works per instance (`check.rs`),
/// tests use these passes.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct Report {
    /// (plugin name, outcome) per VST2 instance found.
    pub items: Vec<(String, How)>,
}

#[cfg(test)]
impl Report {
    pub fn converted(&self) -> usize {
        self.items.iter().filter(|(_, h)| matches!(h, How::Known(_) | How::Generic { .. } | How::Relinked(_) | How::Reidentified)).count()
    }
}

pub fn wrapper_version(data: &[u8]) -> Option<u32> {
    let v = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
    WRAPPER_VERSIONS.contains(&v).then_some(v)
}

/// The records of a VST wrapper's data. Native plugins also use event 213,
/// with their own state; those rarely parse as records and never carry a
/// FourCC or class ID, which every caller checks for.
pub fn plugin_records(data: &[u8]) -> Option<Records> {
    wrapper_version(data)?;
    read_records(&data[4..])
}

pub fn get(records: &[(u32, Vec<u8>)], id: u32) -> Option<&[u8]> {
    records.iter().find(|(i, _)| *i == id).map(|(_, d)| d.as_slice())
}

pub fn text(records: &[(u32, Vec<u8>)], id: u32) -> String {
    get(records, id).map(|b| String::from_utf8_lossy(b).trim_end_matches('\0').to_string()).unwrap_or_default()
}

/// FL's VST2 state: a 21-byte header (byte 4 has bit 3 set for chunk-based
/// plugins), then either the plugin's chunk, or a parameter list:
/// `u32 count, count × f32, u32, program name\0, …`.
pub enum Vst2State<'a> {
    Chunk(&'a [u8]),
    Params { values: Vec<f32>, program: String },
}

pub fn vst2_state(state: &[u8]) -> Option<Vst2State<'_>> {
    if *state.get(4)? & 0x08 != 0 {
        let size = u64::from_le_bytes(state.get(9..17)?.try_into().ok()?) as usize;
        let chunk = state.get(VST2_HEADER..VST2_HEADER.checked_add(size)?)?;
        return (VST2_HEADER + size == state.len()).then_some(Vst2State::Chunk(chunk));
    }
    let n = u32::from_le_bytes(state.get(VST2_HEADER..VST2_HEADER + 4)?.try_into().ok()?) as usize;
    let start = VST2_HEADER + 4;
    let floats = state.get(start..start.checked_add(n.checked_mul(4)?)?)?;
    let values = floats.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    let tail = &state[start + n * 4..];
    let program = tail
        .get(4..)
        .and_then(|t| t.split(|&b| b == 0).next())
        .filter(|t| t.iter().all(|b| (0x20..0x7f).contains(b)))
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .unwrap_or_default();
    Some(Vst2State::Params { values, program })
}

/// What a VST3 instance needs besides its state.
pub struct Target {
    pub class_id: Vec<u8>,
    pub path: String,
    /// VST3 state records to copy (header, parameter table), if known.
    pub state: Option<Records>,
}

impl Target {
    pub fn from_instance(records: &[(u32, Vec<u8>)]) -> Option<Self> {
        let state = get(records, STATE).and_then(|s| s.get(4..)).and_then(read_records);
        Some(Self { class_id: get(records, CLASS_ID)?.to_vec(), path: text(records, PATH), state })
    }
}

/// Builds the VST3 records for one VST2 instance.
fn vst3_records(old: &[(u32, Vec<u8>)], target: &Target, component_state: Vec<u8>, params: Option<u32>) -> Records {
    // Chunk-based plugins don't reveal their parameter count. Record 57's value
    // (0x960) is only a stand-in: an identity table that long is what made
    // Vital open in FL, so it stays until something better is verified.
    let param_count =
        params.or_else(|| get(old, VST2_57).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)).unwrap_or(0);
    let copied = |id| target.state.as_ref().and_then(|s| get(s, id)).map(<[u8]>::to_vec);
    let header = copied(VST3_HEADER).unwrap_or_else(|| {
        let mut h = vec![0; 64];
        h[0] = 1;
        h
    });
    // Steinberg: a VST3 parameter ID must equal the VST2 parameter index.
    let params = copied(VST3_PARAM_IDS).unwrap_or_else(|| {
        let mut t = param_count.to_le_bytes().to_vec();
        (0..param_count).for_each(|i| t.extend(i.to_le_bytes()));
        t
    });
    let state = write_records(
        VST3_STATE_VERSION,
        &[(VST3_HEADER, header), (VST3_COMPONENT, component_state), (VST3_PARAM_IDS, params)],
    );

    let mut out = Records::new();
    for (id, data) in old {
        match *id {
            // FL may already hold a VST3 class ID for a VST2 instance; replaced below.
            FOURCC | VST2_57 | CLASS_ID => {}
            FLAGS => {
                let mut f = data.clone();
                if f.len() >= 4 {
                    f[..4].copy_from_slice(&VST3_FLAG.to_le_bytes());
                }
                out.push((FLAGS, f));
                out.push((CLASS_ID, target.class_id.clone()));
            }
            PATH => out.push((PATH, target.path.as_bytes().to_vec())),
            STATE => {} // written last, like FL does
            _ => out.push((*id, data.clone())),
        }
    }
    if get(&out, CLASS_ID).is_none() {
        out.push((CLASS_ID, target.class_id.clone()));
    }
    out.push((STATE, state));
    out
}

/// 升级插件 from a VST2: the instance as another product's VST3, its chunk
/// through `state`, the target's name. `None` for a parameter-based VST2.
pub fn vst2_to_product(old: &[(u32, Vec<u8>)], target: &Target, state: fn(&[u8]) -> Option<Vec<u8>>, name: &str) -> Option<Records> {
    let Vst2State::Chunk(chunk) = get(old, STATE).and_then(vst2_state)? else { return None };
    let mut records = vst3_records(old, target, state(chunk)?, None);
    for (id, d) in &mut records {
        if *id == NAME {
            *d = name.as_bytes().to_vec();
        }
    }
    Some(records)
}

/// Wraps a VST2 chunk the way Cubase hands VST2 state to a VST3 plugin and
/// the VST3 SDK's `tryVst2StateLoad` reads it: a `VstW` header followed by
/// an FXB chunk bank. All big-endian.
pub fn fxb_chunk_bank(fourcc: u32, chunk: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(chunk.len() + 176);
    b.extend(b"VstW");
    b.extend(8i32.to_be_bytes()); // size of what follows in this header
    b.extend(1i32.to_be_bytes()); // version
    b.extend(0i32.to_be_bytes()); // bypass
    b.extend(b"CcnK");
    b.extend(((152 + chunk.len()) as i32).to_be_bytes()); // bytes after this field
    b.extend(b"FBCh");
    b.extend(2i32.to_be_bytes()); // bank version
    b.extend(fourcc.to_be_bytes());
    b.extend(0i32.to_be_bytes()); // fxVersion (unknown)
    b.extend(1i32.to_be_bytes()); // numPrograms
    b.extend(0i32.to_be_bytes()); // currentProgram
    b.extend([0u8; 124]); // future
    b.extend((chunk.len() as u32).to_be_bytes());
    b.extend(chunk);
    b
}

/// A parameter-based VST2 state as Cubase hands it over: `VstW` header, then
/// an FXB bank (`FxBk`) holding one program (`FxCk`) of float values.
pub fn fxb_param_bank(fourcc: u32, program: &str, values: &[f32]) -> Vec<u8> {
    let mut prg = Vec::new();
    prg.extend(b"CcnK");
    prg.extend(((48 + 4 * values.len()) as i32).to_be_bytes()); // bytes after this field
    prg.extend(b"FxCk");
    prg.extend(1i32.to_be_bytes()); // format version
    prg.extend(fourcc.to_be_bytes());
    prg.extend(0i32.to_be_bytes()); // fxVersion (unknown)
    prg.extend((values.len() as i32).to_be_bytes());
    let mut name = [0u8; 28];
    for (d, s) in name.iter_mut().zip(program.bytes().take(27)) {
        *d = s;
    }
    prg.extend(name);
    values.iter().for_each(|v| prg.extend(v.to_be_bytes()));

    let mut b = Vec::with_capacity(prg.len() + 172);
    b.extend(b"VstW");
    b.extend(8i32.to_be_bytes());
    b.extend(1i32.to_be_bytes());
    b.extend(0i32.to_be_bytes());
    b.extend(b"CcnK");
    b.extend(((148 + prg.len()) as i32).to_be_bytes()); // bytes after this field
    b.extend(b"FxBk");
    b.extend(2i32.to_be_bytes()); // bank version
    b.extend(fourcc.to_be_bytes());
    b.extend(0i32.to_be_bytes()); // fxVersion
    b.extend(1i32.to_be_bytes()); // numPrograms
    b.extend(0i32.to_be_bytes()); // currentProgram
    b.extend([0u8; 124]);
    b.extend(prg);
    b
}

/// The VST3 instances already in a project: the best templates for new ones,
/// as they match the installed plugin version and this machine's paths.
pub fn vst3_instances(project: &Project) -> Vec<Records> {
    project
        .events
        .iter()
        .filter(|e| e.id == PLUGIN_DATA)
        .filter_map(|e| plugin_records(&e.data))
        .filter(|r| get(r, CLASS_ID).is_some() && get(r, FOURCC).is_none())
        .collect()
}

pub fn fourcc(records: &[(u32, Vec<u8>)]) -> Option<u32> {
    get(records, FOURCC).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
}

/// A VST2 instance converted: its VST3 records, how, and the VST3's class ID.
pub struct Upgraded {
    pub records: Records,
    pub how: How,
    pub class_id: Vec<u8>,
}

/// Converts one VST2 instance. `chosen`: a VST3 the user picked instead of
/// the one Blacksmith would find (generic conversion to it). `Err`: why not.
pub fn upgrade_instance(
    lib: &Library,
    old: &[(u32, Vec<u8>)],
    in_project: &[Records],
    chosen: Option<&Entry>,
) -> Result<Upgraded, String> {
    let fourcc = fourcc(old).ok_or("不是 VST2")?;
    let name = text(old, NAME);
    let vendor = text(old, VENDOR);
    let state = get(old, STATE).and_then(vst2_state).ok_or("暂不支持这种存档格式")?;

    let rule = plugins::find(fourcc)
        .filter(|r| chosen.is_none_or(|c| c.class_id == r.class_id))
        .and_then(|r| r.component_state(&state).map(|s| (r, s)));
    if let Some((rule, component)) = rule {
        let target = in_project
            .iter()
            .find(|r| get(r, CLASS_ID) == Some(&rule.class_id[..]))
            .and_then(|r| Target::from_instance(r))
            .unwrap_or_else(|| rule.builtin_target());
        let class_id = target.class_id.clone();
        return Ok(Upgraded { records: vst3_records(old, &target, component, None), how: How::Known(rule.name), class_id });
    }

    // The VST3's real class ID from the plugin library, or the user's pick.
    let table = chosen.or_else(|| lib.find(fourcc));
    let listed = table.map(|k| &k.class_id[..]);
    // FL sometimes already records the VST3 class ID of a VST2 instance
    // (when its plugin manager pairs the two formats).
    let known = get(old, CLASS_ID).filter(|id| id.len() == 16 && chosen.is_none());
    let derived = vst3::class_id_from_vst2(fourcc, &name);
    let in_proj = in_project
        .iter()
        .find(|r| {
            let id = get(r, CLASS_ID);
            if chosen.is_some() {
                return id == listed;
            }
            (listed.is_some() && id == listed)
                || id == known
                || id == Some(&derived[..])
                || (text(r, NAME) == name && text(r, VENDOR) == vendor)
        })
        .and_then(|r| Target::from_instance(r));
    let derived_id = in_proj.is_none() && listed.is_none() && known.is_none();
    let bundle = table.map_or(name.as_str(), |k| k.vst3_file.trim_end_matches(".vst3"));
    let target = in_proj.unwrap_or_else(|| Target {
        class_id: listed.or(known).map(<[u8]>::to_vec).unwrap_or(derived),
        path: vst3::default_path(bundle, &text(old, PATH)),
        state: None,
    });
    let records = match &state {
        Vst2State::Chunk(chunk) => vst3_records(old, &target, fxb_chunk_bank(fourcc, chunk), None),
        Vst2State::Params { values, program } => {
            vst3_records(old, &target, fxb_param_bank(fourcc, program, values), Some(values.len() as u32))
        }
    };
    Ok(Upgraded { records, how: How::Generic { derived: derived_id }, class_id: target.class_id })
}

#[cfg(test)]
pub fn upgrade(project: &mut Project) -> Report {
    let mut report = Report::default();
    let in_project = vst3_instances(project);
    for event in project.events.iter_mut().filter(|e| e.id == PLUGIN_DATA) {
        let Some(old) = plugin_records(&event.data) else { continue };
        let Some(fourcc) = fourcc(&old) else {
            if get(&old, CLASS_ID).is_some() {
                report.items.push((text(&old, NAME), How::AlreadyVst3));
            }
            continue;
        };
        let name = text(&old, NAME);
        let display = if name.is_empty() { format!("{fourcc:08X}") } else { name };
        match upgrade_instance(crate::library::get(crate::library::Host::Fl), &old, &in_project, None) {
            Ok(up) => {
                let version = wrapper_version(&event.data).unwrap_or(10);
                event.data = write_records(version, &up.records);
                report.items.push((display, up.how));
            }
            Err(why) => report.items.push((display, How::Skipped(why))),
        }
    }
    report
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn project(events: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
        let mut out = b"FLhd\x06\x00\x00\x00\x00\x00\x02\x00\x60\x00".to_vec();
        let mut body = Vec::new();
        for (id, d) in events {
            body.push(id);
            let mut n = d.len();
            loop {
                body.push((n & 0x7f) as u8 | if n > 0x7f { 0x80 } else { 0 });
                n >>= 7;
                if n == 0 {
                    break;
                }
            }
            body.extend(d);
        }
        out.extend(b"FLdt");
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    fn vst2(fourcc: &[u8; 4], name: &str, chunk: &[u8]) -> Vec<u8> {
        let mut state = vec![0xf7, 0xff, 0xff, 0xff, 0x0d, 0xfe, 0xff, 0xff, 0xff];
        state.extend((chunk.len() as u64).to_le_bytes());
        state.extend([0; 4]);
        state.extend(chunk);
        write_records(
            KIND_VST,
            &[
                (WINDOW, b"win".to_vec()),
                (FLAGS, vec![0; 16]),
                (FOURCC, fourcc.to_vec()),
                (NAME, name.as_bytes().to_vec()),
                (PATH, b"C:\\x.dll".to_vec()),
                (STATE, state),
                (VST2_57, 3u32.to_le_bytes().to_vec()),
            ],
        )
    }

    #[test]
    fn serum_uses_its_known_rule() {
        let bytes = project(vec![(201, b"wrapper".to_vec()), (PLUGIN_DATA, vst2(b"XsfX", "Serum", b"patch"))]);
        assert_eq!(Project::parse(&bytes).unwrap().to_bytes(), bytes);
        let mut p = Project::parse(&bytes).unwrap();
        let report = upgrade(&mut p);
        assert_eq!(report.items, [("Serum".to_string(), How::Known("Serum"))]);

        let r = plugin_records(&p.events[1].data).unwrap();
        assert_eq!(get(&r, CLASS_ID), Some(&b"XTSVsfsXerum\0\0\0\0"[..]));
        assert_eq!(get(&r, FOURCC), None);
        assert_eq!(get(&r, VST2_57), None);
        assert_eq!(get(&r, WINDOW), Some(&b"win"[..]));
        let st = read_records(&get(&r, STATE).unwrap()[4..]).unwrap();
        assert_eq!(get(&st, VST3_COMPONENT), Some(&b"patch"[..]));
        assert_eq!(st.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1, 3, 4]);
    }

    #[test]
    fn generic_uses_a_vst3_instance_in_the_project() {
        let class_id = vst3::class_id_from_vst2(u32::from_le_bytes(*b"ooFX"), "Foo");
        let vst3 = write_records(
            KIND_VST,
            &[(CLASS_ID, class_id.clone()), (NAME, b"Foo".to_vec()), (PATH, b"C:\\Foo.vst3".to_vec())],
        );
        let bytes = project(vec![(PLUGIN_DATA, vst2(b"ooFX", "Foo", b"abc")), (PLUGIN_DATA, vst3)]);
        let mut p = Project::parse(&bytes).unwrap();
        assert_eq!(upgrade(&mut p).items, [("Foo".to_string(), How::Generic { derived: false }), ("Foo".to_string(), How::AlreadyVst3)]);
        let r = plugin_records(&p.events[0].data).unwrap();
        assert_eq!(get(&r, CLASS_ID), Some(&class_id[..]));
        assert_eq!(get(&r, PATH), Some(&b"C:\\Foo.vst3"[..]));
        let st = read_records(&get(&r, STATE).unwrap()[4..]).unwrap();
        let fxb = get(&st, VST3_COMPONENT).unwrap();
        assert_eq!(&fxb[..4], b"VstW");
        assert_eq!(&fxb[16..20], b"CcnK");
        assert_eq!(&fxb[fxb.len() - 3..], b"abc");
        // parameter table: count 3, ids 0,1,2
        assert_eq!(get(&st, VST3_PARAM_IDS).unwrap().len(), 16);
    }

    #[test]
    fn generic_keeps_one_class_id_and_handles_parameter_state() {
        // A parameter-based VST2 (like FabFilter Pro-R) that FL already paired
        // with a VST3 class ID, plus that VST3 in the project.
        let class_id = b"0123456789abcdef".to_vec();
        let mut state = vec![0xf7, 0xff, 0xff, 0xff, 0x05, 0xfe, 0xff, 0xff, 0xff];
        state.extend([0u8; 12]);
        state.extend(2u32.to_le_bytes());
        state.extend(0.5f32.to_le_bytes());
        state.extend(1.0f32.to_le_bytes());
        state.extend(1u32.to_le_bytes());
        state.extend(b"Default Setting\0\x01\0\0\0");
        let vst2 = write_records(
            KIND_VST,
            &[
                (FLAGS, vec![0; 16]),
                (FOURCC, b"rRPF".to_vec()),
                (CLASS_ID, class_id.clone()),
                (NAME, b"Pro-R".to_vec()),
                (STATE, state),
            ],
        );
        let vst3 = write_records(KIND_VST, &[(CLASS_ID, class_id.clone()), (NAME, b"Pro-R".to_vec()), (PATH, b"C:\\R.vst3".to_vec())]);
        let bytes = project(vec![(PLUGIN_DATA, vst2), (PLUGIN_DATA, vst3)]);
        let mut p = Project::parse(&bytes).unwrap();
        assert_eq!(upgrade(&mut p).items[0], ("Pro-R".to_string(), How::Generic { derived: false }));

        let r = plugin_records(&p.events[0].data).unwrap();
        assert_eq!(r.iter().filter(|(i, _)| *i == CLASS_ID).count(), 1);
        let st = read_records(&get(&r, STATE).unwrap()[4..]).unwrap();
        let fxb = get(&st, VST3_COMPONENT).unwrap();
        assert_eq!(&fxb[16..20], b"CcnK");
        assert_eq!(&fxb[24..28], b"FxBk");
        assert_eq!(i32::from_be_bytes(fxb[20..24].try_into().unwrap()) as usize, fxb.len() - 24);
        let prg = &fxb[172..];
        assert_eq!(&prg[8..12], b"FxCk");
        assert_eq!(i32::from_be_bytes(prg[4..8].try_into().unwrap()) as usize, prg.len() - 8);
        assert_eq!(&prg[28..43], b"Default Setting");
        assert_eq!(&prg[56..64], [0x3f, 0, 0, 0, 0x3f, 0x80, 0, 0]); // 0.5, 1.0 big-endian
    }

    #[test]
    fn generic_falls_back_to_steinbergs_rule() {
        let bytes = project(vec![(PLUGIN_DATA, vst2(b"ooFX", "Foo", b"x"))]);
        let mut p = Project::parse(&bytes).unwrap();
        assert_eq!(upgrade(&mut p).items, [("Foo".to_string(), How::Generic { derived: true })]);
        let r = plugin_records(&p.events[0].data).unwrap();
        assert_eq!(get(&r, CLASS_ID), Some(&vst3::class_id_from_vst2(u32::from_le_bytes(*b"ooFX"), "Foo")[..]));
        assert_eq!(get(&r, PATH), Some(&b"C:\\Program Files\\Common Files\\VST3\\Foo.vst3"[..]));
    }

    #[test]
    fn ott_gets_its_parameters_as_raw_floats() {
        // OTT's VST2 saves parameters, not a chunk; its VST3 ignores an FxBk bank.
        let mut state = vec![0xf7, 0xff, 0xff, 0xff, 0x05, 0xfe, 0xff, 0xff, 0xff];
        state.extend([0u8; 12]);
        state.extend(19u32.to_le_bytes());
        let values: Vec<f32> = (0..19).map(|i| i as f32 / 100.0 + 0.5).collect();
        values.iter().for_each(|v| state.extend(v.to_le_bytes()));
        state.extend(b"\x01\0\0\0DefaultProgram\0");
        let vst2 = write_records(KIND_VST, &[(FLAGS, vec![0; 16]), (FOURCC, b"TTfX".to_vec()), (NAME, b"OTT".to_vec()), (STATE, state)]);
        let mut p = Project::parse(&project(vec![(PLUGIN_DATA, vst2)])).unwrap();
        assert_eq!(upgrade(&mut p).items, [("OTT".to_string(), How::Known("OTT"))]);

        let r = plugin_records(&p.events[0].data).unwrap();
        assert_eq!(get(&r, CLASS_ID), Some(&b"XTSVTfoTtt\0\0\0\0\0\0"[..]));
        let st = read_records(&get(&r, STATE).unwrap()[4..]).unwrap();
        let comp = get(&st, VST3_COMPONENT).unwrap();
        assert_eq!(comp.len(), 80);
        assert_eq!(&comp[..4], &0.5f32.to_le_bytes());
        assert_eq!(&comp[76..], &0f32.to_le_bytes());
        // The VST3's own parameter table: 0..=19 and 'prog'.
        assert_eq!(get(&st, VST3_PARAM_IDS).unwrap().len(), 88);
    }

    #[test]
    fn reads_newer_wrapper_versions_and_keeps_them() {
        // FL 25 saves wrapper data with version 12 instead of 10.
        let mut data = vst2(b"ooFX", "Foo", b"x");
        data[..4].copy_from_slice(&12u32.to_le_bytes());
        let bytes = project(vec![(PLUGIN_DATA, data)]);
        let mut p = Project::parse(&bytes).unwrap();
        assert_eq!(upgrade(&mut p).converted(), 1);
        assert_eq!(wrapper_version(&p.events[0].data), Some(12));
    }

    #[test]
    fn generic_prefers_the_built_in_id_table() {
        // FabFilter doesn't follow Steinberg's rule; the table has its real ID.
        let bytes = project(vec![(PLUGIN_DATA, vst2(b"p3QF", "FabFilter Pro-Q 3", b"x"))]);
        let mut p = Project::parse(&bytes).unwrap();
        assert_eq!(upgrade(&mut p).items, [("FabFilter Pro-Q 3".to_string(), How::Generic { derived: false })]);
        let r = plugin_records(&p.events[0].data).unwrap();
        let real = [0x71, 0xdb, 0xc4, 0x72, 0x4d, 0x7a, 0x9a, 0x45, 0xb9, 0x7e, 0x51, 0x74, 0x5d, 0x84, 0xb3, 0x9d];
        assert_eq!(get(&r, CLASS_ID), Some(&real[..]));
        assert_eq!(get(&r, PATH), Some(&b"C:\\Program Files\\Common Files\\VST3\\FabFilter Pro-Q 3.vst3"[..]));
    }

    #[test]
    fn fxb_bank_matches_the_sdk_layout() {
        let b = fxb_chunk_bank(u32::from_be_bytes(*b"XfsX"), b"hello");
        // CcnK byteSize = everything after the field
        assert_eq!(i32::from_be_bytes(b[20..24].try_into().unwrap()) as usize, b.len() - 24);
        assert_eq!(&b[32..36], b"XfsX");
        assert_eq!(u32::from_be_bytes(b[172..176].try_into().unwrap()), 5);
    }
}

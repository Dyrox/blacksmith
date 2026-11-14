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
//! | 57 | parameter count               | —                                             |
//! | 53 | 21-byte header + plugin chunk | records: 1 header · 3 component state · 4 parameter ID table |
//!
//! Each VST2 instance is matched in order against:
//! 1. a **known plugin** (`plugins.rs`): exact class ID and a verified state
//!    conversion;
//! 2. the **generic** path: find the VST3 version (a VST3 instance of the same
//!    plugin in this project, else an installed .vst3 — see `vst3.rs`) and hand
//!    it the VST2 state the way Steinberg's spec says hosts must: as an FXB
//!    chunk bank (`VST3::tryVst2StateLoad` in the VST3 SDK).

use crate::flp::{Project, Records, read_records, write_records};
use crate::{plugins, vst3};

const PLUGIN_DATA: u8 = 213;
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
pub const PARAM_COUNT: u32 = 57;

const VST3_STATE_VERSION: u32 = 1;
const VST3_HEADER: u32 = 1;
const VST3_COMPONENT: u32 = 3;
const VST3_PARAM_IDS: u32 = 4;
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
    /// Already a VST3 instance; left as is.
    AlreadyVst3,
    Skipped(String),
}

#[derive(Debug, Default)]
pub struct Report {
    /// (plugin name, outcome) per VST2 instance found.
    pub items: Vec<(String, How)>,
}

impl Report {
    pub fn converted(&self) -> usize {
        self.items.iter().filter(|(_, h)| matches!(h, How::Known(_) | How::Generic { .. })).count()
    }

    /// Instances grouped by (name, outcome), in project order, with counts.
    pub fn groups(&self) -> Vec<(String, How, usize)> {
        let mut out: Vec<(String, How, usize)> = Vec::new();
        for (name, how) in &self.items {
            match out.iter_mut().find(|(n, h, _)| n == name && h == how) {
                Some((_, _, c)) => *c += 1,
                None => out.push((name.clone(), how.clone(), 1)),
            }
        }
        out
    }
}

pub fn plugin_records(data: &[u8]) -> Option<Records> {
    if data.len() < 4 || u32::from_le_bytes(data[..4].try_into().ok()?) != KIND_VST {
        return None;
    }
    read_records(&data[4..])
}

pub fn get(records: &[(u32, Vec<u8>)], id: u32) -> Option<&[u8]> {
    records.iter().find(|(i, _)| *i == id).map(|(_, d)| d.as_slice())
}

fn text(records: &[(u32, Vec<u8>)], id: u32) -> String {
    get(records, id).map(|b| String::from_utf8_lossy(b).trim_end_matches('\0').to_string()).unwrap_or_default()
}

/// FL's VST2 state: a 21-byte header (byte 4 has bit 3 set for chunk-based
/// plugins), then either the plugin's chunk, or a parameter list:
/// `u32 count, count × f32, u32, program name\0, …`.
enum Vst2State<'a> {
    Chunk(&'a [u8]),
    Params { values: Vec<f32>, program: String },
}

fn vst2_state(state: &[u8]) -> Option<Vst2State<'_>> {
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
    let param_count =
        params.or_else(|| get(old, PARAM_COUNT).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)).unwrap_or(0);
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
            FOURCC | PARAM_COUNT | CLASS_ID => {}
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

pub fn upgrade(project: &mut Project) -> Report {
    let mut report = Report::default();

    // VST3 instances already in the project are the best templates: they match
    // the installed plugin version and this machine's paths.
    let in_project: Vec<Records> = project
        .events
        .iter()
        .filter(|e| e.id == PLUGIN_DATA)
        .filter_map(|e| plugin_records(&e.data))
        .filter(|r| get(r, CLASS_ID).is_some() && get(r, FOURCC).is_none())
        .collect();

    for event in project.events.iter_mut().filter(|e| e.id == PLUGIN_DATA) {
        let Some(old) = plugin_records(&event.data) else { continue };
        let Some(fourcc) = get(&old, FOURCC).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else {
            if get(&old, CLASS_ID).is_some() {
                report.items.push((text(&old, NAME), How::AlreadyVst3));
            }
            continue;
        };
        let name = text(&old, NAME);
        let vendor = text(&old, VENDOR);
        let display = if name.is_empty() { format!("{fourcc:08X}") } else { name.clone() };

        let Some(state) = get(&old, STATE).and_then(vst2_state) else {
            report.items.push((display, How::Skipped("暂不支持这种存档格式".into())));
            continue;
        };

        let rule = plugins::find(fourcc).filter(|_| matches!(state, Vst2State::Chunk(_)));
        let (records, how) = if let (Some(rule), Vst2State::Chunk(chunk)) = (rule, &state) {
            let target = in_project
                .iter()
                .find(|r| get(r, CLASS_ID) == Some(&rule.class_id[..]))
                .and_then(|r| Target::from_instance(r))
                .unwrap_or_else(|| rule.builtin_target());
            (vst3_records(&old, &target, (rule.state)(chunk), None), How::Known(rule.name))
        } else {
            // FL sometimes already records the VST3 class ID of a VST2 instance
            // (when its plugin manager pairs the two formats).
            let known = get(&old, CLASS_ID).filter(|id| id.len() == 16);
            let derived = vst3::class_id_from_vst2(fourcc, &name);
            let in_proj = in_project
                .iter()
                .find(|r| {
                    let id = get(r, CLASS_ID);
                    id == known || id == Some(&derived[..]) || (text(r, NAME) == name && text(r, VENDOR) == vendor)
                })
                .and_then(|r| Target::from_instance(r));
            let derived_id = in_proj.is_none() && known.is_none();
            let target = in_proj.unwrap_or_else(|| Target {
                class_id: known.map(<[u8]>::to_vec).unwrap_or(derived),
                path: vst3::default_path(&name, &text(&old, PATH)),
                state: None,
            });
            let generic = How::Generic { derived: derived_id };
            match &state {
                Vst2State::Chunk(chunk) => (vst3_records(&old, &target, fxb_chunk_bank(fourcc, chunk), None), generic),
                Vst2State::Params { values, program } => (
                    vst3_records(&old, &target, fxb_param_bank(fourcc, program, values), Some(values.len() as u32)),
                    generic,
                ),
            }
        };
        event.data = write_records(KIND_VST, &records);
        report.items.push((display, how));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(events: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
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
                (PARAM_COUNT, 3u32.to_le_bytes().to_vec()),
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
        assert_eq!(get(&r, PARAM_COUNT), None);
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
    fn fxb_bank_matches_the_sdk_layout() {
        let b = fxb_chunk_bank(u32::from_be_bytes(*b"XfsX"), b"hello");
        // CcnK byteSize = everything after the field
        assert_eq!(i32::from_be_bytes(b[20..24].try_into().unwrap()) as usize, b.len() - 24);
        assert_eq!(&b[32..36], b"XfsX");
        assert_eq!(u32::from_be_bytes(b[172..176].try_into().unwrap()), 5);
    }
}

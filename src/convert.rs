//! VST2 → VST3 upgrades inside an FL Studio project.
//!
//! FL keeps each wrapped plugin in a `PLUGIN_DATA` event: a u32 kind (10 for
//! VST) followed by records (see `flp::read_records`). The ones we touch:
//!
//! | id | VST2                         | VST3                                    |
//! |----|------------------------------|-----------------------------------------|
//! | 51 | FourCC                       | —                                       |
//! | 52 | —                            | 16-byte class ID                        |
//! | 55 | path to the .dll / .vst      | path to the .vst3                       |
//! | 57 | parameter count              | —                                       |
//! | 53 | 21-byte header + plugin chunk| records: 1 header, 3 plugin chunk, 4 parameter ID table |
//!
//! For plugins whose VST3 build reads the same chunk as its VST2 build (Serum
//! does), an upgrade is: take a VST3 instance of that plugin as a template,
//! keep this instance's window record, and drop the VST2 chunk into record 3.

use crate::flp::{Project, read_records, write_records};

const PLUGIN_DATA: u8 = 213;
const KIND_VST: u32 = 10;

const WINDOW: u32 = 2;
const FOURCC: u32 = 51;
const CLASS_ID: u32 = 52;
const STATE: u32 = 53;
const VST3_CHUNK: u32 = 3;
const VST2_HEADER: usize = 21;

struct Plugin {
    name: &'static str,
    /// FourCC as FL stores it (little-endian).
    fourcc: [u8; 4],
    /// VST3 class ID as FL stores it.
    class_id: [u8; 16],
    /// A VST3 wrapper record set with an empty chunk, used when the project
    /// has no VST3 instance of this plugin to copy from.
    template: &'static [u8],
}

const PLUGINS: &[Plugin] = &[Plugin {
    name: "Serum",
    fourcc: *b"XsfX",
    class_id: *b"XTSVsfsXerum\0\0\0\0",
    template: include_bytes!("../assets/serum-vst3-template.bin"),
}];

#[derive(Debug, Default)]
pub struct Report {
    /// (plugin name, how many instances were upgraded)
    pub upgraded: Vec<(&'static str, usize)>,
}

impl Report {
    pub fn total(&self) -> usize {
        self.upgraded.iter().map(|(_, n)| n).sum()
    }
}

fn plugin_records(data: &[u8]) -> Option<Vec<(u32, Vec<u8>)>> {
    if data.len() < 4 || u32::from_le_bytes(data[..4].try_into().ok()?) != KIND_VST {
        return None;
    }
    read_records(&data[4..])
}

fn get(records: &[(u32, Vec<u8>)], id: u32) -> Option<&[u8]> {
    records.iter().find(|(i, _)| *i == id).map(|(_, d)| d.as_slice())
}

pub fn upgrade(project: &mut Project) -> Result<Report, String> {
    let mut report = Report::default();
    for plugin in PLUGINS {
        // Prefer a VST3 instance already in this project: it matches the
        // installed plugin version and the paths on this machine.
        let template = project
            .events
            .iter()
            .filter(|e| e.id == PLUGIN_DATA)
            .filter_map(|e| plugin_records(&e.data))
            .find(|r| get(r, CLASS_ID) == Some(&plugin.class_id))
            .or_else(|| plugin_records(plugin.template))
            .ok_or("built-in template is damaged")?;
        let state = read_records(get(&template, STATE).and_then(|s| s.get(4..)).unwrap_or_default())
            .ok_or("template state is damaged")?;
        let state_version = &get(&template, STATE).unwrap()[..4];

        let mut n = 0;
        for event in project.events.iter_mut().filter(|e| e.id == PLUGIN_DATA) {
            let Some(records) = plugin_records(&event.data) else { continue };
            if get(&records, FOURCC) != Some(&plugin.fourcc) {
                continue;
            }
            let Some(old) = get(&records, STATE) else { continue };
            let size = old.get(9..17).map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize);
            let Some(chunk) = size.and_then(|s| old.get(VST2_HEADER..VST2_HEADER + s)) else {
                return Err(format!("unrecognised {} VST2 state", plugin.name));
            };

            let new_state: Vec<(u32, Vec<u8>)> = state
                .iter()
                .map(|(id, d)| (*id, if *id == VST3_CHUNK { chunk.to_vec() } else { d.clone() }))
                .collect();
            let new_state = write_records(u32::from_le_bytes(state_version.try_into().unwrap()), &new_state);
            let new: Vec<(u32, Vec<u8>)> = template
                .iter()
                .map(|(id, d)| match *id {
                    STATE => (*id, new_state.clone()),
                    WINDOW => (*id, get(&records, WINDOW).unwrap_or(d).to_vec()),
                    _ => (*id, d.clone()),
                })
                .collect();
            event.data = write_records(KIND_VST, &new);
            n += 1;
        }
        if n > 0 {
            report.upgraded.push((plugin.name, n));
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(events: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
        let mut out = b"FLhd\x06\x00\x00\x00\x00\x00\x02\x00\x60\x00".to_vec();
        let mut body = Vec::new();
        for (id, d) in events {
            body.push(id);
            body.push(d.len() as u8); // < 128 in these tests
            body.extend(d);
        }
        out.extend(b"FLdt");
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn upgrades_serum_and_keeps_the_chunk() {
        let chunk = b"serum patch bytes".to_vec();
        let mut state = vec![0xf7, 0xff, 0xff, 0xff, 0x0d, 0xfe, 0xff, 0xff, 0xff];
        state.extend((chunk.len() as u64).to_le_bytes());
        state.extend([0; 4]);
        state.extend(&chunk);
        let vst2 = write_records(
            KIND_VST,
            &[(WINDOW, b"win".to_vec()), (FOURCC, b"XsfX".to_vec()), (STATE, state), (57, vec![0x60, 9, 0, 0])],
        );
        let bytes = project(vec![(201, b"wrapper".to_vec()), (PLUGIN_DATA, vst2)]);
        assert_eq!(Project::parse(&bytes).unwrap().to_bytes(), bytes);

        let mut p = Project::parse(&bytes).unwrap();
        let report = upgrade(&mut p).unwrap();
        assert_eq!(report.total(), 1);

        let r = plugin_records(&p.events[1].data).unwrap();
        assert_eq!(get(&r, CLASS_ID), Some(&b"XTSVsfsXerum\0\0\0\0"[..]));
        assert_eq!(get(&r, FOURCC), None);
        assert_eq!(get(&r, WINDOW), Some(&b"win"[..]));
        let st = read_records(&get(&r, STATE).unwrap()[4..]).unwrap();
        assert_eq!(get(&st, VST3_CHUNK), Some(&chunk[..]));
        assert_eq!(st.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1, 3, 4]);
    }

    #[test]
    fn leaves_other_projects_alone() {
        let bytes = project(vec![(201, b"x".to_vec())]);
        let mut p = Project::parse(&bytes).unwrap();
        assert_eq!(upgrade(&mut p).unwrap().total(), 0);
        assert_eq!(p.to_bytes(), bytes);
    }
}

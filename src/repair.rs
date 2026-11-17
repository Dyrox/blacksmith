//! 修复未识别插件: VST3 instances FL can't find although the plugin is installed.
//!
//! A project from someone else (or an older version of a plugin) can store a
//! VST3 class ID that the installed plugin doesn't use; FL then reports the
//! plugin missing. For every VST3 instance whose class ID isn't in the library
//! (`library.rs`: what FL can load here):
//!
//! 1. an official rule's alias (`plugins.rs`, e.g. the older OTT VST3): new
//!    class ID, state converted, the current build's parameter table;
//! 2. else a library plugin with the same name and vendor: its class ID. The
//!    state is left as it is — whether the installed build reads it isn't known.

use crate::convert::{CLASS_ID, How, NAME, PATH, STATE, Target, VENDOR, VST3_COMPONENT, VST3_PARAM_IDS, get, text};
#[cfg(test)]
use crate::convert::{FOURCC, PLUGIN_DATA, Report, plugin_records, wrapper_version};
#[cfg(test)]
use crate::flp::Project;
use crate::flp::{Records, read_records, write_records};
use crate::library::Library;
use crate::plugins;

#[cfg(test)]
pub fn repair(project: &mut Project) -> Report {
    let mut report = Report::default();
    // VST3 instances of current builds already in the project: the best source
    // of a parameter table for a relinked instance.
    let in_project: Vec<Records> = project
        .events
        .iter()
        .filter(|e| e.id == PLUGIN_DATA)
        .filter_map(|e| plugin_records(&e.data))
        .filter(|r| get(r, CLASS_ID).is_some() && get(r, FOURCC).is_none())
        .collect();

    for event in project.events.iter_mut().filter(|e| e.id == PLUGIN_DATA) {
        let Some(old) = plugin_records(&event.data) else { continue };
        // VST2 instances are the converter's job.
        if get(&old, FOURCC).is_some() || get(&old, CLASS_ID).is_none() {
            continue;
        }
        let (records, how) = repair_instance(crate::library::get(crate::library::Host::Fl), &old, &in_project);
        if let Some(records) = records {
            let version = wrapper_version(&event.data).unwrap_or(10);
            event.data = write_records(version, &records);
        }
        report.items.push((text(&old, NAME), how));
    }
    report
}

/// What repairing one VST3 instance does: new records if anything changes.
pub fn repair_instance(lib: &Library, old: &[(u32, Vec<u8>)], in_project: &[Records]) -> (Option<Records>, How) {
    let Some(id) = get(old, CLASS_ID) else { return (None, How::NotInLibrary) };
    if let Some((rule, alias)) = plugins::find_alias(id) {
        return match relink(old, rule, alias, in_project) {
            Some(r) => (Some(r), How::Relinked(rule.name)),
            None => (None, How::Skipped("旧版 VST3 的存档无法识别".into())),
        };
    }
    if lib.is_installed(id)
        || plugins::RULES.iter().any(|r| r.class_id[..] == *id)
        || lib.is_unscanned_installed(&text(old, PATH), &text(old, NAME), &text(old, VENDOR))
    {
        return (None, How::Recognized);
    }
    match lib.matching(&text(old, NAME), &text(old, VENDOR))[..] {
        [] => (None, How::NotInLibrary),
        [k] => (Some(with_class_id(old, &k.class_id)), How::Reidentified),
        _ => (None, How::Skipped("插件库里有几个同名插件".into())),
    }
}

/// A VST3 instance pointed at another installed VST3 (the user's pick): its
/// class ID changes, its state stays.
pub fn retarget(old: &[(u32, Vec<u8>)], class_id: &[u8]) -> Records {
    with_class_id(old, class_id)
}

fn with_class_id(old: &[(u32, Vec<u8>)], class_id: &[u8]) -> Records {
    old.iter().map(|(i, d)| (*i, if *i == CLASS_ID { class_id.to_vec() } else { d.clone() })).collect()
}

/// Moves a VST3 instance from an older build's class ID to the current one:
/// new ID, state converted by the rule, the current build's parameter table.
fn relink(old: &[(u32, Vec<u8>)], rule: &plugins::Rule, alias: &plugins::Alias, in_project: &[Records]) -> Option<Records> {
    let target = target_in(in_project, &rule.class_id).unwrap_or_else(|| rule.builtin_target());
    move_vst3(old, &target, alias.state, None)
}

/// A VST3 instance of this class ID already in the project, as a target.
pub fn target_in(in_project: &[Records], class_id: &[u8]) -> Option<Target> {
    in_project.iter().find(|r| get(r, CLASS_ID) == Some(class_id)).and_then(|r| Target::from_instance(r))
}

/// Moves a VST3 instance to `target`'s class ID: its component state through
/// `state`, the target's parameter table. `name`: also take the target's name
/// and path (another product, not another build of the same one).
pub fn move_vst3(
    old: &[(u32, Vec<u8>)],
    target: &Target,
    state: fn(&[u8]) -> Option<Vec<u8>>,
    name: Option<&str>,
) -> Option<Records> {
    let saved = get(old, STATE)?;
    let records = read_records(saved.get(4..)?)?;
    let component = state(get(&records, VST3_COMPONENT)?)?;
    let params = target.state.as_ref().and_then(|s| get(s, VST3_PARAM_IDS));
    let new_state: Records = records
        .iter()
        .map(|(id, d)| match *id {
            VST3_COMPONENT => (*id, component.clone()),
            VST3_PARAM_IDS => (*id, params.map_or_else(|| d.clone(), <[u8]>::to_vec)),
            _ => (*id, d.clone()),
        })
        .collect();
    let version = u32::from_le_bytes(saved.get(..4)?.try_into().ok()?);
    let mut out = with_class_id(old, &target.class_id);
    for (id, d) in &mut out {
        match (*id, name) {
            (STATE, _) => *d = write_records(version, &new_state),
            (NAME, Some(n)) => *d = n.as_bytes().to_vec(),
            (PATH, Some(_)) if !target.path.is_empty() => *d = target.path.as_bytes().to_vec(),
            _ => {}
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::tests::project;
    use crate::convert::{FLAGS, VST3_HEADER};

    fn vst3(class_id: &[u8], name: &str, vendor: &str, state: Vec<u8>) -> Vec<u8> {
        write_records(
            11,
            &[
                (FLAGS, 7u32.to_le_bytes().to_vec()),
                (CLASS_ID, class_id.to_vec()),
                (NAME, name.as_bytes().to_vec()),
                (VENDOR, vendor.as_bytes().to_vec()),
                (STATE, state),
            ],
        )
    }

    #[test]
    fn old_ott_vst3_is_relinked() {
        // Older OTT VST3: class ID 'XESV…', 19 floats, table 0..=18 + 'prog'.
        let comp: Vec<u8> = (0..19).flat_map(|i| (i as f32 / 40.0).to_le_bytes()).collect();
        let mut table = 20u32.to_le_bytes().to_vec();
        (0..19u32).chain([u32::from_be_bytes(*b"prog")]).for_each(|i| table.extend(i.to_le_bytes()));
        let state = write_records(1, &[(VST3_HEADER, vec![1; 64]), (VST3_COMPONENT, comp), (VST3_PARAM_IDS, table)]);
        let data = vst3(b"XESVTfoTtt\0\0\0\0\0\0", "OTT", "Xfer Records", state);
        let mut p = Project::parse(&project(vec![(PLUGIN_DATA, data)])).unwrap();
        let report = repair(&mut p);
        assert_eq!(report.items, [("OTT".to_string(), How::Relinked("OTT"))]);
        assert_eq!(report.converted(), 1);

        assert_eq!(wrapper_version(&p.events[0].data), Some(11));
        let r = plugin_records(&p.events[0].data).unwrap();
        assert_eq!(get(&r, CLASS_ID), Some(&b"XTSVTfoTtt\0\0\0\0\0\0"[..]));
        assert_eq!(get(&r, FLAGS), Some(&7u32.to_le_bytes()[..]));
        let st = read_records(&get(&r, STATE).unwrap()[4..]).unwrap();
        let comp = get(&st, VST3_COMPONENT).unwrap();
        assert_eq!(comp.len(), 80);
        assert_eq!(&comp[4..8], &(1.0f32 / 40.0).to_le_bytes());
        assert_eq!(get(&st, VST3_PARAM_IDS).unwrap().len(), 88);
        assert_eq!(get(&st, VST3_HEADER), Some(&[1u8; 64][..]));
    }

    #[test]
    fn upgrade_moves_to_another_product() {
        // Kontakt (6) → Kontakt 8: new ID, name and path, the state as it was,
        // Kontakt 8's parameter table.
        let up = crate::upgrades::from_vst3(b"NTSV$ikDontakt\0\0").unwrap();
        let state = write_records(1, &[(VST3_HEADER, vec![1; 64]), (VST3_COMPONENT, b"hsin...".to_vec()), (VST3_PARAM_IDS, vec![0; 8])]);
        let mut old = read_records(&vst3(b"NTSV$ikDontakt\0\0", "Kontakt", "Native Instruments", state)[4..]).unwrap();
        old.push((PATH, b"/Library/Audio/Plug-Ins/VST3/Kontakt.vst3".to_vec()));
        let new = move_vst3(&old, &up.builtin_target().unwrap(), up.state, Some(up.to_name)).unwrap();
        assert_eq!(get(&new, CLASS_ID), Some(&up.to[..]));
        assert_eq!(get(&new, NAME), Some(&b"Kontakt 8"[..]));
        assert!(get(&new, PATH).unwrap().ends_with(b"Kontakt 8.vst3"));
        let st = read_records(&get(&new, STATE).unwrap()[4..]).unwrap();
        assert_eq!(get(&st, VST3_COMPONENT), Some(&b"hsin..."[..]));
        assert!(get(&st, VST3_PARAM_IDS).unwrap().len() > 8);
    }

    #[test]
    fn unknown_id_gets_the_library_id_by_name() {
        let state = write_records(1, &[(VST3_COMPONENT, b"keep me".to_vec())]);
        let unknown = vst3(&[7; 16], "FabFilter Pro-Q 3", "FabFilter", state.clone());
        let fine = vst3(b"XTSVsfsXerum\0\0\0\0", "Serum", "Xfer Records", state.clone());
        let stranger = vst3(&[9; 16], "Nobody Knows", "Nowhere", state.clone());
        let mut p = Project::parse(&project(vec![(PLUGIN_DATA, unknown), (PLUGIN_DATA, fine), (PLUGIN_DATA, stranger)])).unwrap();
        let report = repair(&mut p);
        assert_eq!(
            report.items,
            [
                ("FabFilter Pro-Q 3".to_string(), How::Reidentified),
                ("Serum".to_string(), How::Recognized),
                ("Nobody Knows".to_string(), How::NotInLibrary),
            ]
        );
        let r = plugin_records(&p.events[0].data).unwrap();
        let real = [0x71, 0xdb, 0xc4, 0x72, 0x4d, 0x7a, 0x9a, 0x45, 0xb9, 0x7e, 0x51, 0x74, 0x5d, 0x84, 0xb3, 0x9d];
        assert_eq!(get(&r, CLASS_ID), Some(&real[..]));
        assert_eq!(get(&r, STATE), Some(&state[..]));
        // Untouched instances stay byte for byte.
        assert_eq!(plugin_records(&p.events[2].data).unwrap(), read_records(&vst3(&[9; 16], "Nobody Knows", "Nowhere", state)[4..]).unwrap());
    }
}

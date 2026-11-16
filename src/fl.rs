//! FL Studio projects (`.flp`) for 检查工程: every plugin instance's status
//! against FL's plugin library, the suggested fix, and applying the choices.
//! Instances are converted by `convert.rs` (VST2 → VST3) and `repair.rs`
//! (VST3 IDs).

use crate::check::{self, Choice, Choices, Finding, Fix, Format, Plugin, Status};
use crate::convert::{self, CLASS_ID, How, NAME, PLUGIN_DATA, STATE, VENDOR, get, plugin_records, text, wrapper_version};
use crate::flp::{Project, Records, write_records};
use crate::library::{Entry, Library};
use crate::{plugins, repair, upgrades};

/// The plugin an instance names.
fn plugin_of(records: &[(u32, Vec<u8>)]) -> Option<Plugin> {
    let (format, id) = match convert::fourcc(records) {
        Some(f) => (Format::Vst2, f.to_be_bytes().to_vec()),
        None => (Format::Vst3, get(records, CLASS_ID)?.to_vec()),
    };
    Some(Plugin { format, id, name: text(records, NAME), vendor: text(records, VENDOR) })
}

fn name_of(lib: &Library, class_id: &[u8]) -> Option<String> {
    lib.by_class_id(class_id)
        .map(|k| k.name.to_string())
        .or_else(|| plugins::RULES.iter().find(|r| r.class_id[..] == *class_id).map(|r| r.name.to_string()))
}

/// Status and suggested fix for one instance.
fn assess(lib: &Library, old: &[(u32, Vec<u8>)], plugin: &Plugin, in_project: &[Records]) -> (Status, Option<Fix>) {
    match plugin.format {
        Format::Vst2 => {
            let fourcc = u32::from_be_bytes(plugin.id[..].try_into().unwrap());
            let fix = convert::upgrade_instance(lib, old, in_project, None).ok().map(|up| Fix::Upgrade {
                target: name_of(lib, &up.class_id).unwrap_or_else(|| plugin.name.clone()),
                installed: lib.is_installed(&up.class_id),
                how: up.how,
            });
            (Status::Vst2 { installed: lib.vst2_installed(fourcc) }, fix)
        }
        Format::Vst3 => {
            let (status, fix) = match repair::repair_instance(lib, old, in_project).1 {
                How::Recognized => (Status::Installed, None),
                How::Relinked(rule) => (Status::Mismatch, Some(Fix::Relink { rule })),
                How::Reidentified => {
                    let target = lib.matching(&plugin.name, &plugin.vendor).first().map(|k| k.name.to_string());
                    (Status::Mismatch, Some(Fix::Rename { target: target.unwrap_or_default() }))
                }
                _ => (Status::Missing, None),
            };
            // A newer product that's installed here beats a rename.
            match upgrades::from(&plugin.id).filter(|u| u.installed_in(lib)) {
                Some(u) => (status, Some(Fix::Migrate { to: u.to_name, verified: u.verified })),
                None => (status, fix),
            }
        }
    }
}

/// Every plugin the project uses, once per plugin with its instance count.
pub fn analyse(lib: &Library, project: &Project) -> Vec<Finding> {
    let in_project = convert::vst3_instances(project);
    let mut out: Vec<Finding> = Vec::new();
    for event in project.events.iter().filter(|e| e.id == PLUGIN_DATA) {
        let Some(old) = plugin_records(&event.data) else { continue };
        let Some(plugin) = plugin_of(&old) else { continue };
        if let Some(f) = out.iter_mut().find(|f| f.plugin == plugin) {
            f.instances += 1;
            continue;
        }
        let (status, fix) = assess(lib, &old, &plugin, &in_project);
        out.push(Finding { plugin, status, fix, instances: 1 });
    }
    out
}

/// The new records for one instance under the user's choices, if it changes.
fn change(lib: &Library, old: &[(u32, Vec<u8>)], plugin: &Plugin, choice: Option<Choice>, in_project: &[Records]) -> Option<Records> {
    let (_, fix) = assess(lib, old, plugin, in_project);
    let decision = check::decide(fix.as_ref(), choice)?;
    let records = match (decision.pick, plugin.format) {
        (None, Format::Vst3) if matches!(fix, Some(Fix::Migrate { .. })) => {
            let up = upgrades::from(&plugin.id)?;
            let target = repair::target_in(in_project, &up.to).unwrap_or_else(|| up.builtin_target());
            repair::move_vst3(old, &target, up.state, Some(up.to_name))?
        }
        (Some(class_id), Format::Vst2) => {
            let target: &Entry = lib.by_class_id(&class_id)?;
            convert::upgrade_instance(lib, old, in_project, Some(target)).ok()?.records
        }
        (Some(class_id), Format::Vst3) => repair::retarget(old, &class_id),
        (None, Format::Vst2) => convert::upgrade_instance(lib, old, in_project, None).ok()?.records,
        (None, Format::Vst3) => repair::repair_instance(lib, old, in_project).0?,
    };
    // Without its state record FL loads the plugin with its defaults (FL's own
    // plugin presets, `.fst`, carry none either).
    Some(if decision.reset { records.into_iter().filter(|(id, _)| *id != STATE).collect() } else { records })
}

/// Applies the choices (and the default fixes); returns how many instances changed.
pub fn apply(lib: &Library, project: &mut Project, choices: &Choices) -> usize {
    let in_project = convert::vst3_instances(project);
    let mut changed = 0;
    for event in project.events.iter_mut().filter(|e| e.id == PLUGIN_DATA) {
        let Some(old) = plugin_records(&event.data) else { continue };
        let Some(plugin) = plugin_of(&old) else { continue };
        if let Some(records) = change(lib, &old, &plugin, choices.get(&plugin).copied(), &in_project) {
            let version = wrapper_version(&event.data).unwrap_or(10);
            event.data = write_records(version, &records);
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::pending;
    use crate::convert::tests::project;
    use crate::library::{Host, get as library};
    use crate::convert::{FLAGS, FOURCC, STATE, VST3_COMPONENT};

    fn vst3(class_id: &[u8], name: &str, vendor: &str) -> Vec<u8> {
        let state = write_records(1, &[(VST3_COMPONENT, b"keep".to_vec())]);
        write_records(
            12,
            &[
                (FLAGS, 7u32.to_le_bytes().to_vec()),
                (CLASS_ID, class_id.to_vec()),
                (NAME, name.as_bytes().to_vec()),
                (VENDOR, vendor.as_bytes().to_vec()),
                (STATE, state),
            ],
        )
    }

    fn serum_vst2() -> Vec<u8> {
        let mut state = vec![0xf7, 0xff, 0xff, 0xff, 0x0d, 0xfe, 0xff, 0xff, 0xff];
        state.extend(5u64.to_le_bytes());
        state.extend([0; 4]);
        state.extend(b"patch");
        write_records(
            10,
            &[(FLAGS, vec![0; 16]), (FOURCC, b"XsfX".to_vec()), (NAME, b"Serum".to_vec()), (VENDOR, b"Xfer Records".to_vec()), (STATE, state)],
        )
    }

    fn sample() -> Project {
        Project::parse(&project(vec![
            (PLUGIN_DATA, serum_vst2()),
            (PLUGIN_DATA, serum_vst2()),
            (PLUGIN_DATA, vst3(b"XTSVsfsXerum\0\0\0\0", "Serum", "Xfer Records")),
            (PLUGIN_DATA, vst3(&[7; 16], "FabFilter Pro-Q 3", "FabFilter")),
            (PLUGIN_DATA, vst3(&[9; 16], "RealLPC", "Nowhere")),
        ]))
        .unwrap()
    }

    #[test]
    fn findings_name_status_and_fix() {
        let f = analyse(library(Host::Fl), &sample());
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].instances, 2);
        assert_eq!(f[0].status, Status::Vst2 { installed: None });
        assert!(matches!(&f[0].fix, Some(Fix::Upgrade { how: How::Known("Serum"), installed: true, .. })));
        assert_eq!(f[1].status, Status::Installed);
        assert_eq!(f[2].status, Status::Mismatch);
        assert_eq!(f[2].fix, Some(Fix::Rename { target: "FabFilter Pro-Q 3".into() }));
        assert_eq!(f[3].status, Status::Missing);
        assert_eq!(f[3].fix, None);
        // Only the official Serum upgrade is on by default; the rename is unverified.
        assert_eq!(pending(&f, &Choices::new()), 2);
    }

    #[test]
    fn choices_turn_fixes_off_and_pick_targets() {
        let f = analyse(library(Host::Fl), &sample());
        let mut choices = Choices::new();
        choices.insert(f[2].plugin.clone(), Choice::Off);
        let pick = *b"XTSVsfsXerum\0\0\0\0";
        choices.insert(f[3].plugin.clone(), Choice::Pick { id: pick, reset: true });
        assert_eq!(pending(&f, &choices), 3);

        let mut p = sample();
        assert_eq!(apply(library(Host::Fl), &mut p, &choices), 3);
        let ids: Vec<Vec<u8>> =
            p.events.iter().map(|e| get(&plugin_records(&e.data).unwrap(), CLASS_ID).unwrap().to_vec()).collect();
        assert_eq!(ids[0], pick); // Serum VST2 → VST3
        assert_eq!(ids[3], [7; 16]); // unticked
        assert_eq!(ids[4], pick); // picked
        // …with its state dropped (reset), so FL opens it with defaults.
        assert_eq!(get(&plugin_records(&p.events[4].data).unwrap(), STATE), None);
        assert!(get(&plugin_records(&p.events[0].data).unwrap(), STATE).is_some());
        let after = analyse(library(Host::Fl), &p);
        assert!(after.iter().all(|f| f.plugin.format == Format::Vst3));
    }
}

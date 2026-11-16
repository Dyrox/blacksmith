//! `blacksmith --dump`: a developer's view of the VST plugins in a project.
//!
//! - `--dump a.flp` lists every VST instance: format, IDs, path, state.
//! - `--dump before.flp after.flp` checks a conversion. `before` is the
//!   original project; `after` is its converted copy opened in FL and saved
//!   again. On that save each loaded VST3 writes its own state, so comparing it
//!   with the original VST2 state shows whether the plugin took the settings.
//!   Instances are paired in project order.

use std::path::{Path, PathBuf};

use crate::convert::{
    CLASS_ID, FLAGS, FOURCC, NAME, PATH, STATE, VST3_COMPONENT, VST3_PARAM_IDS, Vst2State, get, plugin_records,
    text, vst2_state, wrapper_version,
};
use crate::flp::{Project, Records, read_records};

const PLUGIN_DATA: u8 = 213;

struct Plugin {
    version: u32,
    records: Records,
}

impl Plugin {
    fn name(&self) -> String {
        text(&self.records, NAME)
    }
    fn is_vst2(&self) -> bool {
        get(&self.records, FOURCC).is_some()
    }
    fn vst3_state(&self) -> Option<Records> {
        get(&self.records, STATE).and_then(|s| s.get(4..)).and_then(read_records)
    }
    fn component(&self) -> Option<Vec<u8>> {
        self.vst3_state().and_then(|s| get(&s, VST3_COMPONENT).map(<[u8]>::to_vec))
    }
}

fn load(path: &Path) -> Result<Vec<Plugin>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let project = Project::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(project
        .events
        .iter()
        .filter(|e| e.id == PLUGIN_DATA)
        .filter_map(|e| {
            let records = plugin_records(&e.data)?;
            (get(&records, FOURCC).is_some() || get(&records, CLASS_ID).is_some())
                .then(|| Plugin { version: wrapper_version(&e.data).unwrap_or(0), records })
        })
        .collect())
}

pub fn run(args: &[PathBuf]) -> Result<(), String> {
    match args {
        [one] => {
            list(&load(one)?);
            Ok(())
        }
        [before, after] => {
            compare(&load(before)?, &load(after)?);
            Ok(())
        }
        _ => Err("用法：blacksmith --dump a.flp            列出插件\n      blacksmith --dump 原工程.flp FL另存.flp   检查转换".into()),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// First four bytes as text when they look like a format tag ("FFBS", "VstW").
fn tag(b: &[u8]) -> String {
    match b.get(..4) {
        Some(t) if t.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'!') => String::from_utf8_lossy(t).into_owned(),
        _ => "-".into(),
    }
}

fn list(plugins: &[Plugin]) {
    for (i, p) in plugins.iter().enumerate() {
        let r = &p.records;
        let flags = get(r, FLAGS).and_then(|f| f.first()).copied().unwrap_or(0);
        if p.is_vst2() {
            let fourcc = get(r, FOURCC).map(|b| String::from_utf8_lossy(&b.iter().rev().copied().collect::<Vec<_>>()).into_owned());
            let state = match get(r, STATE).and_then(vst2_state) {
                Some(Vst2State::Chunk(c)) => format!("chunk {} B [{}]", c.len(), tag(c)),
                Some(Vst2State::Params { values, program }) => format!("{} 个参数, 程序 \"{program}\"", values.len()),
                None => "无法解析".into(),
            };
            println!("{i:3}  VST2  {}  v{}  flags {flags}  ID {}  {state}", p.name(), p.version, fourcc.unwrap_or_default());
        } else {
            let st = p.vst3_state().unwrap_or_default();
            let comp = get(&st, VST3_COMPONENT).unwrap_or_default();
            let params = get(&st, VST3_PARAM_IDS).and_then(|t| t.get(..4)).map_or(0, |n| u32::from_le_bytes(n.try_into().unwrap()));
            println!(
                "{i:3}  VST3  {}  v{}  flags {flags}  ID {}  component {} B [{}]  参数表 {params}",
                p.name(),
                p.version,
                hex(get(r, CLASS_ID).unwrap_or_default()),
                comp.len(),
                tag(comp),
            );
        }
        println!("       {}", text(r, PATH));
    }
}

/// The plugin data inside a `VstW` wrapper: the chunk of an `FBCh` bank, or
/// the values of the first program of an `FxBk` bank.
enum Unwrapped<'a> {
    Chunk(&'a [u8]),
    Params(Vec<f32>),
}

fn unwrap_vstw(b: &[u8]) -> Option<Unwrapped<'_>> {
    if b.get(..4)? != b"VstW" || b.get(16..20)? != b"CcnK" {
        return None;
    }
    match b.get(24..28)? {
        b"FBCh" => {
            let n = u32::from_be_bytes(b.get(172..176)?.try_into().ok()?) as usize;
            Some(Unwrapped::Chunk(b.get(176..176 + n)?))
        }
        b"FxBk" => {
            let prg = b.get(172..)?;
            let n = i32::from_be_bytes(prg.get(24..28)?.try_into().ok()?) as usize;
            let vals = prg.get(56..56 + 4 * n)?;
            Some(Unwrapped::Params(vals.chunks_exact(4).map(|v| f32::from_be_bytes(v.try_into().unwrap())).collect()))
        }
        _ => None,
    }
}

/// Share of `a`'s distinct 8-byte pieces that also occur in `b`.
fn overlap(a: &[u8], b: &[u8]) -> f32 {
    use std::collections::HashSet;
    let pieces = |x: &[u8]| x.windows(8).map(|w| w.to_vec()).collect::<HashSet<_>>();
    let (a, b) = (pieces(a), pieces(b));
    if a.is_empty() {
        return 0.0;
    }
    a.intersection(&b).count() as f32 / a.len() as f32
}

/// Little-endian floats, when `b` looks like a plain parameter list.
fn as_floats(b: &[u8]) -> Option<Vec<f32>> {
    if b.is_empty() || b.len() % 4 != 0 {
        return None;
    }
    let v: Vec<f32> = b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    v.iter().all(|x| x.is_finite() && (-0.001..=1.001).contains(x)).then_some(v)
}

fn compare_params(before: &[f32], after: &[f32]) -> String {
    let diffs: Vec<String> = before
        .iter()
        .zip(after)
        .enumerate()
        .filter(|(_, (a, b))| (*a - *b).abs() > 1e-4)
        .map(|(i, (a, b))| format!("#{i} {a:.3}→{b:.3}"))
        .collect();
    let n = before.len().min(after.len());
    if diffs.is_empty() {
        format!("✓ {n} 个参数全部一致")
    } else {
        format!("✗ {} / {n} 个参数变了：{}", diffs.len(), diffs.join(", "))
    }
}

fn verdict(before: &Plugin, after: &Plugin) -> String {
    let Some(old) = get(&before.records, STATE).and_then(vst2_state) else {
        return "? 原 VST2 存档无法解析".into();
    };
    let Some(comp) = after.component() else {
        return "? 另存后没有 VST3 存档".into();
    };
    // Still exactly what Blacksmith wrote: the plugin never re-saved it.
    match (&old, unwrap_vstw(&comp)) {
        (Vst2State::Chunk(c), Some(Unwrapped::Chunk(inner))) if inner == *c => {
            return "· 存档原样未动：插件没加载（缺失？），或它自己就存 VST2 格式".into();
        }
        (Vst2State::Params { values, .. }, Some(Unwrapped::Params(v))) if *values == v => {
            return "· 存档原样未动：插件没加载（缺失？）".into();
        }
        _ => {}
    }
    match old {
        Vst2State::Params { values, .. } => match unwrap_vstw(&comp) {
            Some(Unwrapped::Params(v)) => compare_params(&values, &v),
            _ => match as_floats(&comp) {
                Some(v) => compare_params(&values, &v),
                None => format!("? 插件改用自己的格式 [{}] {} B，无法自动比较参数", tag(&comp), comp.len()),
            },
        },
        Vst2State::Chunk(c) => {
            let inner = match unwrap_vstw(&comp) {
                Some(Unwrapped::Chunk(i)) => i,
                _ => &comp,
            };
            if inner == c {
                return "✓ 与原存档完全相同".into();
            }
            let same_format = tag(c) != "-" && tag(c) == tag(inner);
            let pct = 100.0 * overlap(c, inner);
            match (same_format, pct) {
                (true, p) if p >= 50.0 => format!("✓ 同一格式 [{}]，内容重合 {p:.0}%（插件重新保存过）", tag(c)),
                (_, p) if p >= 80.0 => format!("✓ 内容重合 {p:.0}%（插件重新保存过）"),
                (true, p) => format!("? 同一格式 [{}]，内容重合 {p:.0}%，需在 FL 里对比", tag(c)),
                (_, p) => format!(
                    "? 插件改用自己的格式 [{}→{}]，内容重合 {p:.0}%，需在 FL 里对比",
                    tag(c),
                    tag(inner)
                ),
            }
        }
    }
}

fn compare(before: &[Plugin], after: &[Plugin]) {
    if before.len() != after.len() {
        println!("注意：插件数量不同（{} → {}），按顺序配对可能错位\n", before.len(), after.len());
    }
    for (i, (b, a)) in before.iter().zip(after).enumerate() {
        let what = match (b.is_vst2(), a.is_vst2()) {
            (true, false) => verdict(b, a),
            (true, true) => "  仍是 VST2（没转换）".into(),
            (false, false) => "  原本就是 VST3".into(),
            (false, true) => "? VST3 变成了 VST2，配对可能错位".into(),
        };
        let names = if b.name() == a.name() { b.name() } else { format!("{} / {}", b.name(), a.name()) };
        println!("{i:3}  {names:24}  {what}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{fxb_chunk_bank, fxb_param_bank};

    #[test]
    fn unwraps_what_the_converter_writes() {
        let fourcc = u32::from_be_bytes(*b"XfsX");
        assert!(matches!(unwrap_vstw(&fxb_chunk_bank(fourcc, b"hello")), Some(Unwrapped::Chunk(b"hello"))));
        match unwrap_vstw(&fxb_param_bank(fourcc, "P", &[0.25, 1.0])) {
            Some(Unwrapped::Params(v)) => assert_eq!(v, [0.25, 1.0]),
            _ => panic!("FxBk not unwrapped"),
        }
    }

    #[test]
    fn parameter_differences_are_listed() {
        assert_eq!(compare_params(&[0.5, 0.2], &[0.5, 0.2, 0.0]), "✓ 2 个参数全部一致");
        assert_eq!(compare_params(&[0.614, 0.5], &[1.0, 0.5]), "✗ 1 / 2 个参数变了：#0 0.614→1.000");
    }
}

//! `blacksmith --dump`: a developer's view of the VST plugins in a project.
//!
//! - `--dump a.flp` lists every VST instance: format, IDs, path, state.
//! - `--dump before.flp after.flp` checks a conversion. `before` is the
//!   original project; `after` is its converted copy opened in FL and saved
//!   again. On that save each loaded VST3 writes its own state, so comparing it
//!   with the original VST2 state shows whether the plugin took the settings.
//!   Instances are paired in project order.
//! - `--dump a.als` lists every plugin in a Live set with its saved state,
//!   then compares instances of the same VST3: a converted instance re-saved
//!   by Live next to one set up by hand on the same preset should match.
//! - `--dump before.als after.als` pairs instances in order and says what
//!   changed (a converted set and Live's re-save of it).

use std::path::{Path, PathBuf};

use crate::convert::{
    CLASS_ID, FLAGS, FOURCC, NAME, PATH, STATE, VST3_COMPONENT, VST3_PARAM_IDS, Vst2State, get, plugin_records,
    text, vst2_state, wrapper_version,
};
use crate::als::{self, Saved};
use crate::check::Format;
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
    let live = |p: &PathBuf| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("als"));
    if args.iter().any(live) {
        return match args {
            [one] => {
                list_set(&load_set(one)?);
                Ok(())
            }
            [before, after] => {
                compare_sets(&load_set(before)?, &load_set(after)?);
                Ok(())
            }
            _ => Err("用法：blacksmith --dump a.als              列出插件并对比同一插件的实例\n      blacksmith --dump 转换后.als Live另存.als   按顺序对比".into()),
        };
    }
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

// ------------------------------------------------------------ Live sets

fn load_set(path: &Path) -> Result<Vec<Saved>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(als::saved(&als::Set::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?))
}

/// A Live-style FUID: `5653545275756d76-616c-…`.
fn fuid(class_id: &[u8]) -> String {
    let mut b = class_id.to_vec();
    if b.len() == 16 {
        b[..4].reverse();
        b[4..6].reverse();
        b[6..8].reverse();
    }
    hex(&b)
}

/// FabFilter's older VST3 state: `FabF` `u32 2`, name, `u32 0`, `u32 n`, n
/// plain values (f32 LE), then a short tail.
fn fabf(b: &[u8]) -> Option<(String, Vec<f32>)> {
    if b.get(..4)? != b"FabF" {
        return None;
    }
    let u32_at = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize);
    let len = u32_at(8)?;
    let name = String::from_utf8_lossy(b.get(12..12 + len)?).into_owned();
    let n = u32_at(12 + len + 4)?;
    let at = 12 + len + 8;
    let values = b.get(at..at + 4 * n)?.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect();
    Some((name, values))
}

/// Live's parameter buffer for a VST2 (`FxBk`): program name (28 bytes), then f32 LE values.
fn live_params(b: &[u8]) -> Option<(String, Vec<f32>)> {
    let (name, values) = (b.get(..28)?, b.get(28..)?);
    if values.len() % 4 != 0 {
        return None;
    }
    let name = String::from_utf8_lossy(name.split(|&c| c == 0).next()?).into_owned();
    Some((name, values.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect()))
}

/// What a saved state holds, in a few words.
fn describe(s: &Saved) -> String {
    let b = &s.state;
    if b.is_empty() {
        return "无存档".into();
    }
    if s.plugin.format == Format::Vst2 && s.kind == "FxBk" {
        if let Some((name, v)) = live_params(b) {
            return format!("参数 {} 个，程序 \"{name}\"", v.len());
        }
    }
    if let Some((name, v)) = fabf(b) {
        return format!("[FabF] 程序 \"{name}\"，{} 个值", v.len());
    }
    match unwrap_vstw(b) {
        Some(Unwrapped::Chunk(c)) => format!("[VstW 交接] chunk {} B [{}]", c.len(), tag(c)),
        Some(Unwrapped::Params(v)) => format!("[VstW 交接] {} 个参数", v.len()),
        None => format!("{} B [{}]", b.len(), tag(b)),
    }
}

/// How `b`'s state compares with `a`'s (two instances of one VST3).
fn same_plugin(a: &Saved, b: &Saved) -> String {
    if a.state == b.state {
        return "✓ 存档完全相同".into();
    }
    // Plugins that save in the hand-over format themselves (JUCE with VST2
    // compatibility): compare what's inside.
    if let (Some(Unwrapped::Chunk(x)), Some(Unwrapped::Chunk(y))) = (unwrap_vstw(&a.state), unwrap_vstw(&b.state)) {
        return match tail(x, y) {
            Some(t) => format!("✓ VstW 里的 chunk 相同{t}"),
            None => format!("? VstW 里的 chunk 不同，内容重合 {:.0}%", 100.0 * overlap(x, y)),
        };
    }
    if b.state.starts_with(b"VstW") != a.state.starts_with(b"VstW") {
        return "· 一个还是 Blacksmith 写的交接格式：Live 没重新保存，或插件没加载".into();
    }
    if let Some(t) = tail(&a.state, &b.state) {
        return format!("✓ 相同{t}");
    }
    match (fabf(&a.state), fabf(&b.state)) {
        (Some((na, va)), Some((nb, vb))) => {
            let diffs: Vec<String> = va
                .iter()
                .zip(&vb)
                .enumerate()
                .filter(|(_, (x, y))| (*x - *y).abs() > 1e-4 * x.abs().max(1.0))
                .map(|(i, (x, y))| format!("#{i} {x:.4}→{y:.4}"))
                .collect();
            let n = va.len().min(vb.len());
            let names = if na == nb { String::new() } else { format!("（程序 \"{na}\" / \"{nb}\"）") };
            match diffs.len() {
                0 if va.len() == vb.len() => format!("✓ {n} 个值全部一致{names}"),
                0 => format!("? 前 {n} 个值一致，个数不同（{} / {}）{names}", va.len(), vb.len()),
                k => {
                    let shown: Vec<String> = diffs.into_iter().take(8).collect();
                    format!("✗ {k} / {n} 个值不同{names}：{}", shown.join(", "))
                }
            }
        }
        _ => {
            let same_format = tag(&a.state) != "-" && tag(&a.state) == tag(&b.state);
            let pct = 100.0 * overlap(&a.state, &b.state);
            format!("{}内容重合 {pct:.0}%（{} B / {} B）", if same_format { "同一格式，" } else { "" }, a.state.len(), b.state.len())
        }
    }
}

/// When one is the other plus a tail (JUCE's optional `JUCEPrivateData`):
/// what the tail is ("" when they're equal); `None` otherwise.
fn tail(a: &[u8], b: &[u8]) -> Option<String> {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if !long.starts_with(short) {
        return None;
    }
    let rest = &long[short.len()..];
    let text: String = rest.iter().skip_while(|&&c| c == 0).take_while(|c| c.is_ascii_graphic()).map(|&c| c as char).collect();
    Some(match (rest.len(), text.is_empty()) {
        (0, _) => String::new(),
        (n, true) => format!("，一个多了 {n} B 结尾"),
        (n, false) => format!("，一个多了 {n} B 结尾（{text}）"),
    })
}

fn line(i: usize, s: &Saved) -> String {
    let id = match s.plugin.format {
        Format::Vst2 => format!("{} {}", String::from_utf8_lossy(&s.plugin.id), s.kind),
        Format::Vst3 => fuid(&s.plugin.id),
    };
    let format = if s.plugin.format == Format::Vst2 { "VST2" } else { "VST3" };
    let ctrl = if s.controller.is_empty() { String::new() } else { format!("  控制器 {} B", s.controller.len()) };
    format!("{i:3}  {format}  {:22} {:18} {id}  {}{ctrl}", s.plugin.name, s.track, describe(s))
}

fn list_set(all: &[Saved]) {
    for (i, s) in all.iter().enumerate() {
        println!("{}", line(i, s));
    }
    // Instances of the same VST3, each against the first.
    let mut seen: Vec<&[u8]> = Vec::new();
    let mut header = false;
    for s in all.iter().filter(|s| s.plugin.format == Format::Vst3) {
        if seen.contains(&&s.plugin.id[..]) {
            continue;
        }
        seen.push(&s.plugin.id);
        let group: Vec<(usize, &Saved)> = all.iter().enumerate().filter(|(_, o)| o.plugin.id == s.plugin.id).collect();
        if group.len() < 2 {
            continue;
        }
        if !header {
            println!("\n同一 VST3 的实例对比（都和第一个比）：");
            header = true;
        }
        let (first, a) = group[0];
        for (i, b) in &group[1..] {
            println!("  {:22} #{first} ↔ #{i}  {}", a.plugin.name, same_plugin(a, b));
        }
    }
}

fn compare_sets(before: &[Saved], after: &[Saved]) {
    if before.len() != after.len() {
        println!("注意：插件数量不同（{} → {}），按顺序配对可能错位\n", before.len(), after.len());
    }
    for (i, (b, a)) in before.iter().zip(after).enumerate() {
        let what = if b.plugin.format != a.plugin.format || b.plugin.id != a.plugin.id {
            format!("换了插件：{} → {}", describe(b), describe(a))
        } else if b.state == a.state {
            "存档没变".into()
        } else {
            format!("Live 重新保存：{}", same_plugin(b, a))
        };
        println!("{i:3}  {:22} {what}", a.plugin.name);
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

    #[test]
    fn live_states_compare() {
        let mut ff = b"FabF\x02\0\0\0\x05\0\0\0Angry\0\0\0\0\x02\0\0\0".to_vec();
        ff.extend([0.5f32.to_le_bytes(), (-36.0f32).to_le_bytes()].concat());
        ff.extend([1, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(fabf(&ff), Some(("Angry".to_string(), vec![0.5, -36.0])));
        assert_eq!(tail(b"VC2!", b"VC2!\0\0JUCEPrivateData\0\x01"), Some("，一个多了 19 B 结尾（JUCEPrivateData）".into()));
        assert_eq!(tail(b"same", b"same"), Some(String::new()));
        assert_eq!(tail(b"abc", b"abd"), None);
    }
}

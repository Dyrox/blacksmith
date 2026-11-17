// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod als;
mod arch;
mod check;
mod convert;
mod document;
mod dump;
mod fl;
mod fldb;
mod folders;
mod flp;
mod labs;
mod library;
mod livedb;
mod platform;
mod plugins;
mod repair;
mod style;
mod update;
mod upgrades;
mod view;
mod vst2to3;
mod vst3;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use check::{Choice, Choices, Finding, Fix, Plugin, Status};
use document::Document;
use library::Host;
use iced::theme::Mode;
use iced::widget::operation;
use iced::{Event, Size, Subscription, Task, Theme, event, keyboard, system, window};
use style::*;

fn main() -> iced::Result {
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    // `blacksmith --version`: the release this build is (`dev` for a local build).
    if files.first().is_some_and(|f| f.as_os_str() == "--version") {
        println!("Blacksmith {}", update::current().as_deref().unwrap_or("dev"));
        return Ok(());
    }
    // `blacksmith --dump a.flp [b.flp]` lists plugins, or compares a project
    // with FL's re-save of its fixed copy.
    if files.first().is_some_and(|f| f.as_os_str() == "--dump") {
        if let Err(e) = dump::run(&files[1..]) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    // `blacksmith --library` prints each host's plugin library and where it came from.
    if files.first().is_some_and(|f| f.as_os_str() == "--library") {
        for host in [Host::Fl, Host::Live] {
            let lib = library::get(host);
            match lib.source() {
                library::Source::Host(d) => eprintln!("{}：插件数据库 {}", host.name(), d.display()),
                library::Source::Folders => eprintln!("{}：没找到插件数据库，用插件文件夹 + 内置列表", host.name()),
            }
            let scan = lib.scan();
            eprintln!(
                "  插件文件夹 {} 个插件；moduleinfo.json 补上 {} 个 ID；忽略 {} 个已删除的；{} 个文件只有 Intel 版；未扫描：{}",
                scan.on_disk,
                scan.ids_from_files,
                scan.stale,
                lib.intel_count(),
                scan.unregistered.join(", ")
            );
            for k in lib.all() {
                let fourcc = k.fourcc.map_or("-".to_string(), |f| format!("{f:08x}"));
                let id: String = k.class_id.iter().map(|b| format!("{b:02x}")).collect();
                let kind = if k.instrument { "instrument" } else { "effect" };
                let arch = if lib.intel_only(k) { "intel" } else { "" };
                println!("{}\t{fourcc}\t{id}\t{kind}\t{}\t{}\t{}\t{arch}", host.name(), k.vst3_file, k.vendor, k.name);
            }
        }
        return Ok(());
    }
    // `blacksmith a.flp b.als folder …` checks without a window and writes nothing;
    // `--apply` saves the default fixes (like pressing 应用), `--apply-all`
    // every available one, unverified included, keeping settings (dev aid).
    if !files.is_empty() {
        let flag = |f: &PathBuf| f.as_os_str() == "--apply" || f.as_os_str() == "--apply-all";
        let all = files.iter().any(|f| f.as_os_str() == "--apply-all");
        let apply = files.iter().any(flag);
        let files: Vec<PathBuf> = files.into_iter().filter(|f| !flag(f)).collect();
        for f in projects_in(files) {
            let before = match analyse(&f) {
                ProjectState::Checked { findings, .. } => findings,
                other => {
                    println!("{}  {}", f.display(), other.headline());
                    continue;
                }
            };
            let list: Vec<String> = before
                .iter()
                .map(|x| {
                    let fix = x.fix.as_ref().filter(|x| x.default_on()).map(|x| format!(" → {}", fix_label(x))).unwrap_or_default();
                    format!("{}× {} [{}{fix}]", x.instances, x.plugin.name, status_label(&x.status).0)
                })
                .collect();
            let choices: Choices = if all {
                before.iter().filter(|x| x.fix.is_some()).map(|x| (x.plugin.clone(), Choice::On { reset: false })).collect()
            } else {
                Choices::new()
            };
            let state = if apply { save(&f, &choices) } else { analyse(&f) };
            println!("{}  {}  {}", f.display(), state.headline(), list.join(", "));
        }
        return Ok(());
    }

    iced::application(App::boot, App::update, App::view)
        .title("Blacksmith")
        .subscription(App::subscription)
        .theme(|app: &App| if app.dark { Theme::Dark } else { Theme::Light })
        .window(window::Settings {
            size: Size::new(1040.0, 680.0),
            min_size: Some(Size::new(820.0, 520.0)),
            // assets/icon-64.rgba, made by tools/icons.sh from assets/logo.svg.
            icon: window::icon::from_rgba(include_bytes!("../assets/icon-64.rgba").to_vec(), 64, 64).ok(),
            ..Default::default()
        })
        .run()
}

// ------------------------------------------------------------------ work

#[derive(Debug, Clone)]
enum ProjectState {
    /// Read; nothing written yet.
    Checked { host: Host, findings: Vec<Finding> },
    /// Fixes applied and saved to `out`; `findings` are the saved copy's.
    Saved { host: Host, out: PathBuf, changed: usize, findings: Vec<Finding> },
    Failed(String),
}

impl ProjectState {
    fn findings(&self) -> &[Finding] {
        match self {
            ProjectState::Checked { findings, .. } | ProjectState::Saved { findings, .. } => findings,
            ProjectState::Failed(_) => &[],
        }
    }

    /// The host the project belongs to (FL Studio when it couldn't be read).
    fn host(&self) -> Host {
        match self {
            ProjectState::Checked { host, .. } | ProjectState::Saved { host, .. } => *host,
            ProjectState::Failed(_) => Host::Fl,
        }
    }

    fn headline(&self) -> String {
        match self {
            ProjectState::Checked { findings, .. } => match check::pending(findings, &Choices::new()) {
                0 => "没有需要处理的插件".into(),
                n => format!("{n} 处可以处理"),
            },
            ProjectState::Saved { out, changed, .. } => {
                format!("已另存为 {}（改了 {changed} 处）", out.file_name().unwrap_or_default().to_string_lossy())
            }
            ProjectState::Failed(e) => e.clone(),
        }
    }
}

/// Reads a project and lists its plugins. Writes nothing.
fn analyse(path: &Path) -> ProjectState {
    match Document::read(path) {
        Ok(d) => ProjectState::Checked { host: d.host(), findings: d.analyse() },
        Err(e) => ProjectState::Failed(e),
    }
}

/// Applies the choices and saves `<name> (修复).<ext>` next to the project.
/// The original is never touched.
fn save(path: &Path, choices: &Choices) -> ProjectState {
    let mut doc = match Document::read(path) {
        Ok(d) => d,
        Err(e) => return ProjectState::Failed(e),
    };
    let host = doc.host();
    let changed = doc.apply(choices);
    if changed == 0 {
        return ProjectState::Checked { host, findings: doc.analyse() };
    }
    let out = output_path(path);
    match std::fs::write(&out, doc.to_bytes()) {
        Ok(()) => ProjectState::Saved { host, out, changed, findings: doc.analyse() },
        Err(e) => ProjectState::Failed(format!("保存失败：{e}")),
    }
}

/// The projects in `paths`: files as given, folders searched (Backup folders
/// — FL's and Live's —, hidden folders and Blacksmith's own results skipped).
fn projects_in(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if p.is_dir() {
                if depth < 12 && !name.starts_with('.') && !name.eq_ignore_ascii_case("Backup") {
                    walk(&p, out, depth + 1);
                }
            } else if document::is_document(&p) && !is_ours(&name) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(&p, &mut out, 0);
        } else {
            out.push(p);
        }
    }
    out
}

/// `x (修复).flp`, `x (修复) 2.als`, and the older `x (VST3).flp`: copies
/// Blacksmith saved.
fn is_ours(file: &str) -> bool {
    ["修复", "VST3"].iter().any(|s| file.contains(&format!(" ({s}).")) || file.contains(&format!(" ({s}) ")))
}

fn output_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().unwrap_or_default().to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out = dir.join(format!("{stem} (修复).{ext}"));
    let mut i = 2;
    while out.exists() {
        out = dir.join(format!("{stem} (修复) {i}.{ext}"));
        i += 1;
    }
    out
}

/// Opens the file manager with `path` selected.
fn reveal_in_folder(path: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(Path::new("."))).spawn();
}

/// Opens a web page in the default browser.
fn open_url(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Words and colour for a plugin's status.
fn status_label(status: &Status) -> (String, Tone) {
    match status {
        Status::Installed => ("已安装".into(), Tone::Green),
        Status::Mismatch => ("ID 对不上".into(), Tone::Amber),
        Status::Missing => ("未安装".into(), Tone::Red),
        Status::Vst2 { installed: Some(true) } => ("VST2 · 已安装".into(), Tone::Green),
        Status::Vst2 { installed: Some(false) } => ("VST2 · 未安装".into(), Tone::Red),
        Status::Vst2 { installed: None } => ("VST2".into(), Tone::Amber),
        Status::Rosetta => ("仅 Intel · 需 Rosetta".into(), Tone::Amber),
        Status::Deprecated { .. } => ("已停产".into(), Tone::Amber),
    }
}

/// What a fix does, in words.
fn fix_label(fix: &Fix) -> String {
    match fix {
        Fix::Upgrade { target, installed: true, .. } => format!("升级到 {target} VST3"),
        Fix::Upgrade { target, installed: false, .. } => format!("升级到 {target} VST3（这台电脑没装）"),
        Fix::Relink { rule } => format!("改成已装的 {rule}"),
        Fix::Rename { target } => format!("改成已装的 {target}"),
        Fix::Migrate { to, .. } => format!("升级到 {to}（换成新插件）"),
    }
}

// ------------------------------------------------------------------ app

/// Which plugins the table shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Filter {
    /// Anything not installed as is: VST2, wrong ID, missing.
    Attention,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Page {
    Check,
    Library,
    Supported,
}

const SEARCH: &str = "library-search";
const PICK_SEARCH: &str = "pick-search";

/// The project being checked: one at a time.
struct Item {
    path: PathBuf,
    state: ProjectState,
}

struct App {
    dark: bool,
    page: Page,
    hovering: bool,
    project: Option<Item>,
    /// What the user decided per plugin.
    choices: Choices,
    filter: Filter,
    /// The plugin a replacement is being picked for, and the picker's search.
    picking: Option<Plugin>,
    pick_query: String,
    now: Instant,
    /// Animation clocks: launch (first frame), last page switch, last drag-hover change.
    boot_at: Instant,
    page_at: Instant,
    hover_at: Instant,
    started: bool,
    /// When the table last changed; drives its rows' reveal.
    list_at: Instant,
    /// Analyses still running in the background.
    scanning: usize,
    /// Plugin library page: which host's library, search text, open vendors
    /// (lower-case), last vendor opened and when.
    library_host: Host,
    query: String,
    open: HashSet<String>,
    opened: Option<(String, Instant)>,
    /// A newer release on GitHub, once the check at launch finds one.
    update: Option<update::Release>,
}

#[derive(Debug, Clone)]
enum Message {
    Dropped(PathBuf),
    Hover(bool),
    Nav(Page),
    Browse,
    Picked(Option<PathBuf>),
    Analysed(PathBuf, ProjectState),
    Apply,
    Clear,
    Reveal(PathBuf),
    Filter(Filter),
    /// Tick or untick a plugin's fix.
    Tick(Plugin, bool),
    /// Keep (false) or reset (true) a ticked plugin's settings.
    Reset(Plugin, bool),
    /// Open the picker for a plugin; `None` closes it.
    Pick(Option<Plugin>),
    PickSearch(String),
    /// Use this installed VST3 (class ID) for the plugin being picked for.
    PickTarget([u8; 16]),
    Theme(Mode),
    Focused,
    Frame(Instant),
    Search(String),
    FocusSearch,
    LibraryHost(Host),
    ToggleVendor(String),
    ExpandAll(bool),
    /// The update check's answer.
    Update(Option<update::Release>),
    OpenUrl(String),
}

/// Reveal animation timing.
const FADE: f32 = 0.25;
const SLIDE: f32 = 0.5;

fn ease_out_cubic(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// Overshoots past 1 and settles back: the "pop".
fn ease_out_back(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    const C1: f32 = 1.7;
    const C3: f32 = C1 + 1.0;
    1.0 + C3 * (x - 1.0).powi(3) + C1 * (x - 1.0).powi(2)
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        // Debug aid: BLACKSMITH_DARK=1 forces the dark palette.
        let forced = std::env::var("BLACKSMITH_DARK").is_ok();
        let dark = forced || platform::system_dark().unwrap_or(false);
        style::set_dark(dark);
        // Read FL's plugin database off the UI thread while the window opens.
        std::thread::spawn(library::warm);
        let now = Instant::now();
        let mut app = Self {
            dark,
            page: Page::Check,
            hovering: false,
            project: None,
            choices: Choices::new(),
            filter: Filter::Attention,
            picking: None,
            pick_query: String::new(),
            now,
            boot_at: now,
            page_at: now,
            // As if the last hover ended long ago, so nothing swells at launch.
            hover_at: now.checked_sub(Duration::from_secs(10)).unwrap_or(now),
            started: false,
            list_at: now,
            scanning: 0,
            library_host: Host::Fl,
            query: String::new(),
            open: HashSet::new(),
            opened: None,
            update: None,
        };
        // Debug aids: BLACKSMITH_DROP=<file> acts as a drop at launch;
        // BLACKSMITH_PAGE=library|supported opens that page.
        let drop = std::env::var_os("BLACKSMITH_DROP").map(|f| app.analyse(PathBuf::from(f)));
        match std::env::var("BLACKSMITH_PAGE").as_deref() {
            Ok("supported") => app.page = Page::Supported,
            Ok("library") => {
                app.page = Page::Library;
                app.open.extend(["fabfilter".to_string(), "xfer records".to_string()]);
            }
            _ => {}
        }
        let theme = if forced { Task::none() } else { system::theme().map(Message::Theme) };
        // Ask GitHub for a newer release off the UI thread.
        let update = Task::perform(async { update::check() }, Message::Update);
        (app, Task::batch([theme, drop.unwrap_or_else(Task::none), update]))
    }

    /// Reads a project off the UI thread; it replaces the one shown.
    fn analyse(&mut self, path: PathBuf) -> Task<Message> {
        self.go(Page::Check);
        self.now = Instant::now();
        self.scanning += 1;
        Task::perform(
            async move {
                let state = if path.is_dir() {
                    ProjectState::Failed("这是一个文件夹；请拖入一个 .flp 或 .als 工程".into())
                } else {
                    analyse(&path)
                };
                (path, state)
            },
            |(p, s)| Message::Analysed(p, s),
        )
    }

    fn go(&mut self, page: Page) {
        if self.page != page {
            self.page = page;
            self.page_at = Instant::now();
        }
    }

    fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        style::set_dark(dark);
    }

    /// Seconds since `at`, on the animation clock.
    fn since(&self, at: Instant) -> f32 {
        self.now.saturating_duration_since(at).as_secs_f32()
    }

    /// Instances the 应用 button would change.
    fn pending(&self) -> usize {
        match self.project.as_ref().map(|i| &i.state) {
            Some(ProjectState::Checked { findings, .. }) => check::pending(findings, &self.choices),
            _ => 0,
        }
    }

    /// Whether a plugin's fix is on, under the user's choice or by default.
    fn ticked(&self, f: &Finding) -> bool {
        match self.choices.get(&f.plugin) {
            Some(Choice::Off) => false,
            Some(Choice::On { .. } | Choice::Pick { .. }) => true,
            None => f.fix.as_ref().is_some_and(Fix::default_on),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Dropped(path) => {
                self.hovering = false;
                return self.analyse(path);
            }
            Message::Hover(h) => {
                if self.hovering != h {
                    self.hovering = h;
                    self.hover_at = Instant::now();
                    self.now = self.hover_at;
                }
            }
            Message::Nav(page) => {
                self.go(page);
                self.now = Instant::now();
            }
            Message::Browse => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("选择工程")
                            .add_filter("FL Studio / Ableton Live 工程", &document::EXTENSIONS)
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::Picked,
                );
            }
            Message::Picked(None) => {}
            Message::Picked(Some(path)) => return self.analyse(path),
            Message::Analysed(path, state) => {
                self.scanning = self.scanning.saturating_sub(1);
                // A different project starts with fresh choices.
                if self.project.as_ref().is_none_or(|p| p.path != path) {
                    self.choices.clear();
                }
                self.project = Some(Item { path, state });
                self.list_at = Instant::now();
                self.now = self.list_at;
            }
            Message::Apply => {
                if self.pending() > 0 {
                    let item = self.project.as_mut().expect("pending implies a project");
                    item.state = save(&item.path, &self.choices);
                    self.list_at = Instant::now();
                    self.now = self.list_at;
                }
            }
            Message::Clear => {
                self.project = None;
                self.choices.clear();
                self.boot_at = Instant::now(); // replay the drop-zone pop
                self.list_at = self.boot_at;
                self.now = self.boot_at;
            }
            Message::Reveal(path) => reveal_in_folder(&path),
            Message::Update(release) => self.update = release,
            Message::OpenUrl(url) => open_url(&url),
            Message::Filter(filter) => {
                self.filter = filter;
                self.list_at = Instant::now();
                self.now = self.list_at;
            }
            Message::Tick(plugin, on) => {
                let reset = self.choices.get(&plugin).is_some_and(|c| c.reset());
                self.choices.insert(plugin, if on { Choice::On { reset } } else { Choice::Off });
            }
            Message::Reset(plugin, reset) => {
                if let Some(c) = self.choices.get_mut(&plugin) {
                    *c = c.with_reset(reset);
                }
            }
            Message::Pick(plugin) => {
                let open = plugin.is_some();
                self.picking = plugin;
                self.pick_query.clear();
                if open {
                    return operation::focus(PICK_SEARCH);
                }
            }
            Message::PickSearch(q) => self.pick_query = q,
            Message::PickTarget(class_id) => {
                if let Some(plugin) = self.picking.take() {
                    self.choices.insert(plugin, Choice::Pick { id: class_id, reset: false });
                }
            }
            Message::Theme(mode) => {
                if platform::system_dark().is_none() {
                    self.set_dark(mode == Mode::Dark);
                }
            }
            // macOS doesn't deliver appearance changes to the window; re-read on focus.
            Message::Focused => {
                if let Some(d) = platform::system_dark() {
                    self.set_dark(d);
                }
            }
            Message::Frame(now) => {
                // The launch animation starts at the first rendered frame, so a
                // slow GPU start doesn't eat it.
                if !self.started {
                    self.started = true;
                    self.boot_at = now;
                    self.list_at = now;
                }
                self.now = now;
            }
            Message::Search(q) => self.query = q,
            Message::LibraryHost(host) => {
                self.library_host = host;
                self.open.clear();
                self.list_at = Instant::now();
                self.now = self.list_at;
            }
            Message::FocusSearch => {
                self.go(Page::Library);
                self.now = Instant::now();
                return operation::focus(SEARCH);
            }
            Message::ToggleVendor(vendor) => {
                if !self.open.remove(&vendor) {
                    self.open.insert(vendor.clone());
                    self.opened = Some((vendor, Instant::now()));
                    self.now = Instant::now();
                }
            }
            Message::ExpandAll(open) => {
                self.open.clear();
                if open {
                    self.open.extend(library::get(self.library_host).all().iter().map(|k| k.vendor.to_lowercase()));
                }
            }
        }
        Task::none()
    }

    fn animating(&self) -> bool {
        !self.started
            || self.since(self.boot_at) < 0.8
            || self.since(self.page_at) < 0.6
            || self.since(self.hover_at) < 0.6
            || self.since(self.list_at) < 1.2
            || self.opened.as_ref().is_some_and(|(_, at)| self.since(*at) < 0.8)
    }

    /// Scale of the drop-zone icon: pops in at launch, swells while a file hovers.
    fn icon_scale(&self) -> f32 {
        let pop = 0.6 + 0.4 * ease_out_back(self.since(self.boot_at) / 0.6);
        let k = ease_out_back(self.since(self.hover_at) / 0.45);
        let swell = if self.hovering { 1.0 + 0.18 * k } else { 1.18 - 0.18 * k };
        pop * swell
    }

    /// Extra top offset of page content while it slides in.
    fn page_offset(&self) -> f32 {
        16.0 * (1.0 - ease_out_back(self.since(self.page_at) / 0.45))
    }

    fn subscription(&self) -> Subscription<Message> {
        let events = event::listen_with(|event, _, _| match event {
            Event::Window(window::Event::FileDropped(p)) => Some(Message::Dropped(p)),
            Event::Window(window::Event::FileHovered(_)) => Some(Message::Hover(true)),
            Event::Window(window::Event::FilesHoveredLeft) => Some(Message::Hover(false)),
            Event::Window(window::Event::Focused) => Some(Message::Focused),
            Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(keyboard::key::Named::Escape), .. }) => {
                Some(Message::Pick(None))
            }
            // Ctrl/⌘+O opens a project, Ctrl/⌘+Enter applies, Ctrl/⌘+F searches.
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if modifiers.command() => match key.as_ref() {
                keyboard::Key::Character("o") => Some(Message::Browse),
                keyboard::Key::Character("f") => Some(Message::FocusSearch),
                keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Message::Apply),
                _ => None,
            },
            _ => None,
        });
        let frames = if self.animating() { window::frames().map(Message::Frame) } else { Subscription::none() };
        Subscription::batch([events, system::theme_changes().map(Message::Theme), frames])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_searched_for_projects_but_not_backups_or_results() {
        let dir = std::env::temp_dir().join(format!("blacksmith-scan-{}", std::process::id()));
        for f in ["a/song.flp", "a/song (VST3).flp", "a/song (修复) 2.flp", "a/Backup/song (autosave).flp", "b/c/beat.FLP", "b/notes.txt"] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"").unwrap();
        }
        // Compared with `/` on every OS (Windows paths use `\`).
        let found: Vec<String> = projects_in(vec![dir.clone()])
            .iter()
            .map(|p| {
                let rel = p.strip_prefix(&dir).unwrap();
                rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
            })
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found, ["a/song.flp", "b/c/beat.FLP"]);
    }
}

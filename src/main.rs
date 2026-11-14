// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod convert;
mod flp;
mod platform;
mod plugins;
mod style;
mod vst3;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use convert::How;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::theme::Mode;
use iced::widget::{Column, Space, button, center, column, container, row, scrollable};
use iced::{Alignment, Element, Event, Length, Padding, Size, Subscription, Task, Theme, event, keyboard, system, window};
use style::*;

fn main() -> iced::Result {
    // `blacksmith a.flp b.flp …` converts without opening a window.
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if !files.is_empty() {
        for f in &files {
            let s = run(f, true);
            let list = s.groups().iter().map(|(n, h, c)| format!("{c}× {n} [{}]", badge(h).0)).collect::<Vec<_>>();
            println!("{}  {}  {}", f.display(), s.headline(), list.join(", "));
        }
        return Ok(());
    }

    iced::application(App::boot, App::update, App::view)
        .title("Blacksmith")
        .subscription(App::subscription)
        .theme(|app: &App| if app.dark { Theme::Dark } else { Theme::Light })
        .window(window::Settings {
            size: Size::new(880.0, 620.0),
            min_size: Some(Size::new(700.0, 480.0)),
            ..Default::default()
        })
        .run()
}

// ------------------------------------------------------------------ work

type Groups = Vec<(String, How, usize)>;

enum State {
    /// Analysed; `n` instances will be converted when the user asks.
    Ready { groups: Groups, n: usize },
    Done { groups: Groups, out: PathBuf },
    /// Nothing to convert (no VST2, or none convertible).
    Nothing { groups: Groups, note: &'static str },
    Failed(String),
}

impl State {
    fn groups(&self) -> &[(String, How, usize)] {
        match self {
            State::Ready { groups, .. } | State::Done { groups, .. } | State::Nothing { groups, .. } => groups,
            State::Failed(_) => &[],
        }
    }

    fn headline(&self) -> String {
        match self {
            State::Ready { n, .. } => format!("可转换 {n} 个插件"),
            State::Done { out, .. } => format!("已另存为 {}", out.file_name().unwrap_or_default().to_string_lossy()),
            State::Nothing { note, .. } => note.to_string(),
            State::Failed(e) => e.clone(),
        }
    }
}

/// Analyses a project and, if `write`, saves `<name> (VST3).flp` next to it.
/// The original is never touched.
fn run(path: &Path, write: bool) -> State {
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("flp")) {
        return State::Failed("不是 .flp 文件".into());
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return State::Failed(format!("读取失败：{e}")),
    };
    let mut project = match flp::Project::parse(&bytes) {
        Ok(p) if p.to_bytes() == bytes => p,
        Ok(_) => return State::Failed("无法无损读取这个工程".into()),
        Err(e) => return State::Failed(format!("无法读取：{e}")),
    };
    let report = convert::upgrade(&mut project);
    let groups = report.groups();
    let n = report.converted();
    if n == 0 {
        let note = if report.items.iter().any(|(_, h)| matches!(h, How::Skipped(_))) {
            "有 VST2 插件，但都无法转换"
        } else {
            "没有需要转换的 VST2 插件"
        };
        return State::Nothing { groups, note };
    }
    if !write {
        return State::Ready { groups, n };
    }
    let out = output_path(path);
    match std::fs::write(&out, project.to_bytes()) {
        Ok(()) => State::Done { groups, out },
        Err(e) => State::Failed(format!("保存失败：{e}")),
    }
}

fn output_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out = dir.join(format!("{stem} (VST3).flp"));
    let mut i = 2;
    while out.exists() {
        out = dir.join(format!("{stem} (VST3) {i}.flp"));
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

/// Label and colour for a plugin's outcome.
fn badge(how: &How) -> (String, Tone) {
    match how {
        How::Known(_) => ("官方支持".into(), Tone::Green),
        How::Generic { derived: false } => ("通用转换".into(), Tone::Blue),
        How::Generic { derived: true } => ("通用转换 · 推导 ID".into(), Tone::Blue),
        How::AlreadyVst3 => ("已是 VST3".into(), Tone::Grey),
        How::Skipped(why) => (format!("跳过 · {why}"), Tone::Amber),
    }
}

// ------------------------------------------------------------------ app

#[derive(Debug, Clone, Copy, PartialEq)]
enum Page {
    Convert,
    Supported,
}

struct Item {
    path: PathBuf,
    state: State,
    /// When this item (re)appeared; drives its reveal animation.
    shown_at: Instant,
}

struct App {
    dark: bool,
    page: Page,
    hovering: bool,
    items: Vec<Item>,
    now: Instant,
    /// Animation clocks: launch (first frame), last page switch, last drag-hover change.
    boot_at: Instant,
    page_at: Instant,
    hover_at: Instant,
    started: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Dropped(PathBuf),
    Hover(bool),
    Nav(Page),
    Browse,
    Picked(Vec<PathBuf>),
    Convert,
    Clear,
    Remove(usize),
    Reveal(PathBuf),
    Theme(Mode),
    Focused,
    Frame(Instant),
}

/// Reveal animation: card first, then each plugin row a beat later.
const STAGGER: f32 = 0.05;
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
        let now = Instant::now();
        let mut app = Self {
            dark,
            page: Page::Convert,
            hovering: false,
            items: Vec::new(),
            now,
            boot_at: now,
            page_at: now,
            // As if the last hover ended long ago, so nothing swells at launch.
            hover_at: now.checked_sub(Duration::from_secs(10)).unwrap_or(now),
            started: false,
        };
        // Debug aids: BLACKSMITH_DROP=<file> acts as a drop at launch;
        // BLACKSMITH_PAGE=supported opens the supported-plugins page.
        if let Some(f) = std::env::var_os("BLACKSMITH_DROP") {
            app.add(PathBuf::from(f));
        }
        if std::env::var("BLACKSMITH_PAGE").as_deref() == Ok("supported") {
            app.page = Page::Supported;
        }
        (app, if forced { Task::none() } else { system::theme().map(Message::Theme) })
    }

    fn add(&mut self, path: PathBuf) {
        let item = Item { state: run(&path, false), path, shown_at: Instant::now() };
        // Dropping the same file again re-analyses it in place.
        match self.items.iter_mut().find(|i| i.path == item.path) {
            Some(existing) => *existing = item,
            None => self.items.push(item),
        }
        self.go(Page::Convert);
        self.now = Instant::now();
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

    fn ready(&self) -> usize {
        self.items.iter().filter_map(|i| if let State::Ready { n, .. } = i.state { Some(n) } else { None }).sum()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Dropped(path) => {
                self.hovering = false;
                self.add(path);
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
                            .set_title("选择 FL Studio 工程")
                            .add_filter("FL Studio 工程", &["flp"])
                            .pick_files()
                            .await
                            .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::Picked,
                );
            }
            Message::Picked(paths) => paths.into_iter().for_each(|p| self.add(p)),
            Message::Convert => {
                let now = Instant::now();
                for item in &mut self.items {
                    if matches!(item.state, State::Ready { .. }) {
                        item.state = run(&item.path, true);
                        item.shown_at = now; // pop the updated card in again
                    }
                }
                self.go(Page::Convert);
                self.now = now;
            }
            Message::Clear => {
                self.items.clear();
                self.boot_at = Instant::now(); // replay the drop-zone pop
                self.now = self.boot_at;
            }
            Message::Remove(i) => {
                if i < self.items.len() {
                    self.items.remove(i);
                }
            }
            Message::Reveal(path) => reveal_in_folder(&path),
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
                    self.items.iter_mut().for_each(|i| i.shown_at = now);
                }
                self.now = now;
            }
        }
        Task::none()
    }

    fn animating(&self) -> bool {
        let items = self.items.iter().any(|i| {
            let rows = i.state.groups().len() as f32;
            self.since(i.shown_at) < SLIDE + STAGGER * (rows + 1.0) + 0.05
        });
        !self.started
            || items
            || self.since(self.boot_at) < 0.8
            || self.since(self.page_at) < 0.6
            || self.since(self.hover_at) < 0.6
    }

    /// The n-th element of an item's reveal: (opacity, springy slide progress).
    fn reveal(&self, item: &Item, n: usize) -> (f32, f32) {
        let t = self.since(item.shown_at) - STAGGER * n as f32;
        (ease_out_cubic(t / FADE), ease_out_back(t / SLIDE))
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
            // Ctrl/⌘+O adds files, Ctrl/⌘+Enter converts.
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if modifiers.command() => match key.as_ref() {
                keyboard::Key::Character("o") => Some(Message::Browse),
                keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Message::Convert),
                _ => None,
            },
            _ => None,
        });
        let frames = if self.animating() { window::frames().map(Message::Frame) } else { Subscription::none() };
        Subscription::batch([events, system::theme_changes().map(Message::Theme), frames])
    }

    // -------------------------------------------------------------- views

    fn view(&self) -> Element<'_, Message> {
        let page = match self.page {
            Page::Convert => self.convert_page(),
            Page::Supported => self.supported_page(),
        };
        let off = self.page_offset();
        let page = container(page).padding(Padding { top: off.max(0.0), bottom: (-off).max(0.0), ..Padding::ZERO });
        row![
            self.sidebar(),
            container(page).width(Length::Fill).height(Length::Fill).padding([24, 28]).style(|_| background()),
        ]
        .into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let p = p();
        let nav_item = |label: &'static str, glyph: &'static str, page: Page, count: usize| {
            let active = self.page == page;
            let mut line = row![
                container(t(glyph).size(13).color(if active { p.accent } else { p.text_2 }))
                    .width(24)
                    .center_x(24),
                t(label).size(14).font(if active { SEMIBOLD } else { MEDIUM }).color(if active { p.text } else { p.text_2 }),
                Space::new().width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if count > 0 {
                line = line.push(
                    container(t(count.to_string()).size(11).font(SEMIBOLD).color(if active { p.on_accent } else { p.text_2 }))
                        .padding([1, 7])
                        .style(move |_| count_badge(active)),
                );
            }
            button(line).width(Length::Fill).padding([9, 10]).on_press(Message::Nav(page)).style(move |_, s| nav(active, s))
        };
        let logo_row = row![
            container(t("B").size(15).font(BOLD).color(p.panel)).width(30).height(30).center_x(30).center_y(30).style(|_| logo()),
            column![t("Blacksmith").size(15).font(BOLD).color(p.text), t("VST2 → VST3").size(11).color(p.text_3)],
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let shortcut = |keys: &'static str, what: &'static str| {
            row![t(keys).size(11).font(MEDIUM).color(p.text_2), Space::new().width(Length::Fill), t(what).size(11).color(p.text_3)]
        };
        let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };

        container(
            column![
                logo_row,
                Space::new().height(20),
                nav_item("转换工程", "⇄", Page::Convert, self.items.len()),
                nav_item("官方支持", "✓", Page::Supported, plugins::RULES.len()),
                Space::new().height(Length::Fill),
                shortcut(if cfg!(target_os = "macos") { "⌘ O" } else { "Ctrl O" }, "添加文件"),
                shortcut(if cmd == "⌘" { "⌘ ↵" } else { "Ctrl ↵" }, "转换"),
                Space::new().height(6),
                t("FL Studio · .flp").size(11).color(p.text_3),
            ]
            .spacing(4),
        )
        .width(220)
        .height(Length::Fill)
        .padding(Padding { top: 20.0, right: 12.0, bottom: 16.0, left: 12.0 })
        .style(|_| sidebar())
        .into()
    }

    fn header<'a>(&self, title: String, subtitle: String, action: Option<Element<'a, Message>>) -> Element<'a, Message> {
        let p = p();
        let mut r = row![
            column![t(title).size(22).font(BOLD).color(p.text), t(subtitle).size(13).color(p.text_2)]
                .spacing(3)
                .width(Length::Fill),
        ]
        .align_y(Alignment::Center);
        if let Some(a) = action {
            r = r.push(a);
        }
        r.into()
    }

    fn convert_page(&self) -> Element<'_, Message> {
        let p = p();
        let add = button(t("添加文件…").size(13).font(MEDIUM)).padding([9, 16]).on_press(Message::Browse).style(|_, s| secondary(s));

        if self.items.is_empty() {
            let h = self.hovering;
            let d = 76.0 * self.icon_scale();
            let icon = container(t("↓").size(34.0 * d / 76.0).font(BOLD).color(if h { p.on_accent } else { p.accent }))
                .width(d)
                .height(d)
                .center_x(d)
                .center_y(d)
                .style(move |_| icon_circle(d, h));
            let zone = container(center(
                column![
                    container(icon).width(110).height(110).center_x(110).center_y(110),
                    t(if h { "松开即可分析" } else { "把 .flp 文件拖到这里" }).size(18).font(SEMIBOLD).color(p.text),
                    t("先列出工程里的插件，确认后再转换。原文件不会被改动。").size(13).color(p.text_2),
                    Space::new().height(10),
                    button(t("选择文件…").size(14).font(SEMIBOLD))
                        .padding([11, 22])
                        .on_press(Message::Browse)
                        .style(|_, s| primary(s)),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            ))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| drop_zone(h));
            return column![
                self.header("转换工程".into(), "FL Studio 工程里的 VST2 插件换成 VST3".into(), None),
                zone,
            ]
            .spacing(18)
            .into();
        }

        let ready = self.ready();
        let done = self.items.iter().filter(|i| matches!(i.state, State::Done { .. })).count();
        let subtitle = match (ready, done) {
            (0, 0) => "这些工程里没有能升级的 VST2 插件".to_string(),
            (0, d) => format!("已转换 {d} 个工程，结果另存在原文件旁边"),
            (r, _) => format!("{} 个工程 · {r} 个插件待转换", self.items.len()),
        };

        let mut cards = Column::new().spacing(12).padding(Padding { right: 12.0, ..Padding::ZERO });
        for (i, item) in self.items.iter().enumerate() {
            cards = cards.push(self.card(i, item));
        }
        cards = cards.push(
            container(t(if self.hovering { "松开即可添加" } else { "可以继续拖入 .flp 文件" }).size(12).color(if self.hovering {
                p.accent
            } else {
                p.text_3
            }))
            .center_x(Length::Fill)
            .padding(8),
        );

        let convert_label = if ready > 0 { format!("转换 {ready} 个插件") } else { "转换".into() };
        let footer = row![
            button(t("清空").size(13).font(MEDIUM)).padding([11, 14]).on_press(Message::Clear).style(|_, s| ghost(s)),
            Space::new().width(Length::Fill),
            button(t(convert_label).size(14).font(SEMIBOLD).center().width(Length::Fill))
                .width(Length::Fixed(180.0))
                .padding([11, 18])
                .on_press_maybe((ready > 0).then_some(Message::Convert))
                .style(|_, s| primary(s)),
        ]
        .align_y(Alignment::Center);

        column![
            self.header("转换工程".into(), subtitle, Some(add.into())),
            scrollable(cards).direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4))).height(Length::Fill),
            footer,
        ]
        .spacing(16)
        .into()
    }

    fn card<'a>(&self, index: usize, item: &'a Item) -> Element<'a, Message> {
        let p = p();
        let (a, slide) = self.reveal(item, 0);
        let file = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = item.path.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let (chip, tone) = match &item.state {
            State::Ready { .. } => ("待转换", Tone::Blue),
            State::Done { .. } => ("已转换", Tone::Green),
            State::Nothing { .. } => ("无需转换", Tone::Grey),
            State::Failed(_) => ("出错", Tone::Red),
        };
        let remove = button(t("✕").size(13).color(fade(p.text_3, a)).center())
            .width(30)
            .height(30)
            .padding(0)
            .on_press(Message::Remove(index))
            .style(|_, s| ghost(s));
        let headline = match item.state {
            State::Done { .. } => p.green,
            State::Failed(_) => p.red,
            _ => p.text_2,
        };
        let mut body = column![
            row![
                container(t("FL").size(12).font(BOLD).color(fade(p.accent, a)))
                    .width(36)
                    .height(36)
                    .center_x(36)
                    .center_y(36)
                    .style(move |_| tile(a)),
                column![t(file).size(15).font(SEMIBOLD).color(fade(p.text, a)), t(dir).size(11).color(fade(p.text_3, a))]
                    .spacing(1)
                    .width(Length::Fill),
                pill(chip, tone, a),
                remove,
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            t(item.state.headline()).size(12).color(fade(headline, a)),
        ]
        .spacing(8);

        let groups = item.state.groups();
        if !groups.is_empty() {
            let mut list = Column::new().spacing(4);
            for (i, (name, how, count)) in groups.iter().enumerate() {
                let (a, k) = self.reveal(item, i + 1);
                let (label, tone) = badge(how);
                let mut left = row![t(name).size(13).font(MEDIUM).color(fade(p.text, a))].spacing(6).align_y(Alignment::Center);
                if *count > 1 {
                    left = left.push(t(format!("×{count}")).size(12).color(fade(p.text_3, a)));
                }
                list = list.push(
                    container(
                        row![left, Space::new().width(Length::Fill), pill(label, tone, a)]
                            .align_y(Alignment::Center)
                            .padding(Padding { left: 12.0 + 20.0 * (1.0 - k), ..Padding::from([7, 12]) }),
                    )
                    .style(move |_| row_style(a)),
                );
            }
            body = body.push(list);
        } else if !matches!(item.state, State::Failed(_)) {
            body = body.push(t("工程里没有 VST 插件").size(12).color(fade(p.text_3, a)));
        }
        if let State::Done { out, .. } = &item.state {
            body = body.push(row![
                Space::new().width(Length::Fill),
                button(t("在文件夹中显示").size(13).font(MEDIUM))
                    .padding([8, 14])
                    .on_press(Message::Reveal(out.clone()))
                    .style(|_, s| secondary(s)),
            ]);
        }
        // The card drops in from slightly below with a springy settle.
        let lift = 16.0 * (1.0 - slide);
        container(container(body).padding(16).width(Length::Fill).style(move |_| card(a)))
            .padding(Padding { top: lift.max(0.0), bottom: (-lift).max(0.0), ..Padding::ZERO })
            .into()
    }

    fn supported_page(&self) -> Element<'_, Message> {
        let p = p();
        let mut rules = Column::new().spacing(10);
        for r in plugins::RULES {
            rules = rules.push(
                container(
                    row![
                        container(t(&r.name[..1]).size(16).font(BOLD).color(p.accent))
                            .width(40)
                            .height(40)
                            .center_x(40)
                            .center_y(40)
                            .style(|_| tile(1.0)),
                        column![
                            row![t(r.name).size(15).font(SEMIBOLD).color(p.text), t(r.vendor).size(12).color(p.text_3)]
                                .spacing(8)
                                .align_y(Alignment::Center),
                            t(r.note).size(12).color(p.text_2),
                        ]
                        .spacing(3)
                        .width(Length::Fill),
                        pill("官方支持", Tone::Green, 1.0),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding(14)
                .style(|_| card(1.0)),
            );
        }

        let legend = [
            (How::Known(""), "有专门规则、在 FL 里验证过的插件"),
            (How::Generic { derived: false }, "按 VST3 规范转换；VST3 的 ID 来自 FL 自己的记录或工程里已有的 VST3"),
            (How::Generic { derived: true }, "按 VST3 规范转换；ID 按 Steinberg 规则推导，个别厂商（如 FabFilter）不遵守，可能打不开"),
            (How::AlreadyVst3, "已经是 VST3，不动"),
            (How::Skipped(String::new()), "无法转换，原样保留 VST2"),
        ];
        let mut explain = Column::new().spacing(10);
        for (how, text) in legend {
            let (label, tone) = badge(&how);
            explain = explain.push(
                row![
                    container(pill(label.trim_end_matches(" · ").to_string(), tone, 1.0)).width(160),
                    t(text).size(12).color(p.text_2).width(Length::Fill),
                ]
                .align_y(Alignment::Center),
            );
        }

        column![
            self.header("官方支持".into(), "这些插件有专门的转换规则，音色和设置会原样迁移".into(), None),
            scrollable(
                column![
                    rules,
                    Space::new().height(6),
                    t("标签说明").size(14).font(SEMIBOLD).color(p.text),
                    container(explain).padding(16).style(|_| card(1.0)),
                    t("想加新的官方支持：提供插件的 VST2 和 VST3，以及一个两种格式都加载了同一个预设的工程。").size(12).color(p.text_3),
                ]
                .spacing(12)
                .padding(Padding { right: 12.0, ..Padding::ZERO }),
            )
            .direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4)))
            .height(Length::Fill),
        ]
        .spacing(18)
        .into()
    }
}

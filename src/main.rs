// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod convert;
mod flp;
mod platform;
mod plugins;
mod vst3;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use convert::How;
use iced::border::Radius;
use iced::theme::Mode;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::text::Shaping;
use iced::widget::{Column, Space, Text, button, center, column, container, row, scrollable, text};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Font, Length, Padding, Shadow, Size, Subscription, Task,
    Theme, event, font, system, window,
};

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
            size: Size::new(600.0, 560.0),
            min_size: Some(Size::new(440.0, 380.0)),
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

/// Label and colours for a plugin's outcome.
fn badge(how: &How) -> (String, Tone) {
    match how {
        How::Known(_) => ("官方支持".into(), Tone::Green),
        How::Generic => ("通用转换".into(), Tone::Blue),
        How::AlreadyVst3 => ("已是 VST3".into(), Tone::Grey),
        How::Skipped(why) => (format!("跳过 · {why}"), Tone::Warn),
    }
}

// ------------------------------------------------------------------ app

struct Item {
    path: PathBuf,
    state: State,
    /// When this item (re)appeared; drives its reveal animation.
    shown_at: Instant,
}

struct App {
    dark: bool,
    hovering: bool,
    items: Vec<Item>,
    now: Instant,
}

#[derive(Debug, Clone)]
enum Message {
    Dropped(PathBuf),
    Hover(bool),
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

/// Reveal animation: card header first, then each plugin row a beat later.
const STAGGER: f32 = 0.06;
const FADE: f32 = 0.28;

impl App {
    fn boot() -> (Self, Task<Message>) {
        let dark = platform::system_dark().unwrap_or(false);
        let mut app = Self { dark, hovering: false, items: Vec::new(), now: Instant::now() };
        // Debug aid: BLACKSMITH_DROP=<file> behaves as if that file was dropped at launch.
        if let Some(f) = std::env::var_os("BLACKSMITH_DROP") {
            app.add(PathBuf::from(f));
        }
        (app, system::theme().map(Message::Theme))
    }

    fn add(&mut self, path: PathBuf) {
        let item = Item { state: run(&path, false), path, shown_at: Instant::now() };
        // Dropping the same file again re-analyses it in place.
        match self.items.iter_mut().find(|i| i.path == item.path) {
            Some(existing) => *existing = item,
            None => self.items.push(item),
        }
        self.now = Instant::now();
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
            Message::Hover(h) => self.hovering = h,
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
                for item in &mut self.items {
                    if matches!(item.state, State::Ready { .. }) {
                        item.state = run(&item.path, true);
                    }
                }
            }
            Message::Clear => self.items.clear(),
            Message::Remove(i) => {
                if i < self.items.len() {
                    self.items.remove(i);
                }
            }
            Message::Reveal(path) => reveal_in_folder(&path),
            Message::Theme(mode) => {
                if platform::system_dark().is_none() {
                    self.dark = mode == Mode::Dark;
                }
            }
            // macOS doesn't deliver appearance changes to the window; re-read on focus.
            Message::Focused => {
                if let Some(d) = platform::system_dark() {
                    self.dark = d;
                }
            }
            Message::Frame(now) => self.now = now,
        }
        Task::none()
    }

    fn animating(&self) -> bool {
        self.items.iter().any(|i| {
            let rows = i.state.groups().len() as f32;
            self.now.duration_since(i.shown_at) < Duration::from_secs_f32(FADE + STAGGER * (rows + 1.0) + 0.05)
        })
    }

    /// 0 → 1 progress of the n-th element of an item's reveal (eased).
    fn reveal(&self, item: &Item, n: usize) -> f32 {
        let t = self.now.saturating_duration_since(item.shown_at).as_secs_f32() - STAGGER * n as f32;
        let x = (t / FADE).clamp(0.0, 1.0);
        1.0 - (1.0 - x).powi(3)
    }

    fn subscription(&self) -> Subscription<Message> {
        let events = event::listen_with(|event, _, _| match event {
            Event::Window(window::Event::FileDropped(p)) => Some(Message::Dropped(p)),
            Event::Window(window::Event::FileHovered(_)) => Some(Message::Hover(true)),
            Event::Window(window::Event::FilesHoveredLeft) => Some(Message::Hover(false)),
            Event::Window(window::Event::Focused) => Some(Message::Focused),
            _ => None,
        });
        let frames = if self.animating() { window::frames().map(Message::Frame) } else { Subscription::none() };
        Subscription::batch([events, system::theme_changes().map(Message::Theme), frames])
    }

    // -------------------------------------------------------------- views

    fn view(&self) -> Element<'_, Message> {
        let p = if self.dark { &DARK } else { &LIGHT };

        let header = row![
            column![
                t("Blacksmith").size(20).font(BOLD).color(p.text),
                t("FL Studio 工程 · VST2 → VST3").size(12).color(p.text_2),
            ]
            .spacing(2),
            Space::new().width(Length::Fill),
            secondary(p, "选择文件…", Some(Message::Browse)),
        ]
        .align_y(Alignment::Center);

        let content: Element<'_, Message> = if self.items.is_empty() {
            self.empty(p)
        } else {
            let mut cards = Column::new().spacing(10);
            for (i, item) in self.items.iter().enumerate() {
                cards = cards.push(self.card(i, item, p));
            }
            cards = cards.push(
                container(t("拖入更多 .flp 文件").size(12).color(p.text_3)).width(Length::Fill).center_x(Length::Fill).padding(8),
            );
            scrollable(cards)
                .direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4).spacing(6)))
                .height(Length::Fill)
                .into()
        };
        // The whole content area is the drop target; outline it while hovering.
        let hovering = self.hovering;
        let content = container(content).padding(if self.items.is_empty() { 0 } else { 2 }).height(Length::Fill).style(
            move |_| container::Style {
                border: Border {
                    color: if hovering { p.accent } else { Color::TRANSPARENT },
                    width: 1.5,
                    radius: Radius::from(14.0),
                },
                ..Default::default()
            },
        );

        let ready = self.ready();
        let done = self.items.iter().filter(|i| matches!(i.state, State::Done { .. })).count();
        let status = match (self.items.len(), ready, done) {
            (0, ..) => String::new(),
            (n, 0, 0) => format!("{n} 个工程 · 没有可转换的插件"),
            (n, 0, d) => format!("{n} 个工程 · 已转换 {d} 个"),
            (n, r, _) => format!("{n} 个工程 · {r} 个插件待转换"),
        };
        let mut actions = row![].spacing(8);
        if !self.items.is_empty() {
            actions = actions.push(secondary(p, "清空", Some(Message::Clear)));
        }
        let label = if ready > 0 { format!("转换 {ready} 个插件") } else { "转换".into() };
        actions = actions.push(primary(p, label, (ready > 0).then_some(Message::Convert)));
        let footer = row![t(status).size(12).color(p.text_2), Space::new().width(Length::Fill), actions].align_y(Alignment::Center);

        let chrome = p.chrome;
        container(column![header, content, footer].spacing(14))
            .padding(18)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(Background::Color(chrome)), ..Default::default() })
            .into()
    }

    fn empty(&self, p: &'static Palette) -> Element<'_, Message> {
        let (border, bg) = if self.hovering { (p.accent, p.hover) } else { (p.border, p.panel) };
        container(center(
            column![
                container(t("⇣").size(26).color(p.text_2)).padding([6, 16]).style(move |_| container::Style {
                    background: Some(Background::Color(p.line)),
                    border: Border { radius: Radius::from(12.0), ..Default::default() },
                    ..Default::default()
                }),
                Space::new().height(6),
                t("把 .flp 文件拖到这里").size(18).font(SEMIBOLD).color(p.text),
                t("先列出工程里的插件，确认后再转换").size(13).color(p.text_2),
                t("转换后的工程另存在原文件旁边，原文件不动").size(12).color(p.text_3),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { color: border, width: 1.5, radius: Radius::from(14.0) },
            ..Default::default()
        })
        .into()
    }

    fn card<'a>(&self, index: usize, item: &'a Item, p: &'static Palette) -> Element<'a, Message> {
        let a = self.reveal(item, 0);
        let file = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = item.path.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let (chip, tone) = match &item.state {
            State::Ready { .. } => ("待转换", Tone::Blue),
            State::Done { .. } => ("已转换", Tone::Green),
            State::Nothing { .. } => ("无需转换", Tone::Grey),
            State::Failed(_) => ("出错", Tone::Danger),
        };

        let remove = button(t("✕").size(12).color(fade(p.text_3, a)))
            .padding([2, 6])
            .on_press(Message::Remove(index))
            .style(move |_, s| button::Style {
                background: (s == button::Status::Hovered).then_some(Background::Color(p.line)),
                border: Border { radius: Radius::from(6.0), ..Default::default() },
                ..Default::default()
            });
        let title = row![
            t(file).size(15).font(SEMIBOLD).color(fade(p.text, a)),
            Space::new().width(Length::Fill),
            pill(chip, tone, p, a),
            remove,
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let headline_color = match item.state {
            State::Done { .. } => p.green,
            State::Failed(_) => p.danger,
            _ => p.text_2,
        };
        let mut body = column![
            title,
            t(dir).size(11).color(fade(p.text_3, a)),
            t(item.state.headline()).size(12).color(fade(headline_color, a)),
        ]
        .spacing(3);

        let groups = item.state.groups();
        if !groups.is_empty() {
            let mut list = Column::new().spacing(4);
            for (i, (name, how, count)) in groups.iter().enumerate() {
                let a = self.reveal(item, i + 1);
                let (label, tone) = badge(how);
                let mut left = row![t(name).size(13).font(MEDIUM).color(fade(p.text, a))].spacing(6).align_y(Alignment::Center);
                if *count > 1 {
                    left = left.push(t(format!("×{count}")).size(12).color(fade(p.text_2, a)));
                }
                let line = fade(p.line, a);
                list = list.push(
                    container(
                        row![left, Space::new().width(Length::Fill), pill(label, tone, p, a)]
                            .align_y(Alignment::Center)
                            .padding(Padding { left: 10.0 + 12.0 * (1.0 - a), ..Padding::from([6, 10]) }),
                    )
                    .style(move |_| container::Style {
                        background: Some(Background::Color(line)),
                        border: Border { radius: Radius::from(8.0), ..Default::default() },
                        ..Default::default()
                    }),
                );
            }
            body = body.push(Space::new().height(6)).push(list);
        } else if !matches!(item.state, State::Failed(_)) {
            body = body.push(t("工程里没有 VST 插件").size(12).color(fade(p.text_3, a)));
        }

        if let State::Done { out, .. } = &item.state {
            body = body.push(Space::new().height(4)).push(row![
                Space::new().width(Length::Fill),
                secondary(p, "在文件夹中显示", Some(Message::Reveal(out.clone()))),
            ]);
        }

        let border = fade(p.border, a);
        container(body)
            .padding(14)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(p.panel)),
                border: Border { color: border, width: 1.0, radius: Radius::from(12.0) },
                ..Default::default()
            })
            .into()
    }
}

// ------------------------------------------------------------------ widgets

/// Text with full shaping, so Chinese and other scripts fall back to system fonts.
fn t<'a>(s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).shaping(Shaping::Advanced)
}

fn fade(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

fn pill<'a>(label: impl text::IntoFragment<'a>, tone: Tone, p: &'static Palette, a: f32) -> Element<'a, Message> {
    let (fg, bg) = tone.colors(p);
    container(t(label).size(11).font(MEDIUM).color(fade(fg, a)))
        .padding([2, 9])
        .style(move |_| container::Style {
            background: Some(Background::Color(fade(bg, a))),
            border: Border { radius: Radius::from(99.0), ..Default::default() },
            ..Default::default()
        })
        .into()
}

fn primary<'a>(p: &'static Palette, label: impl text::IntoFragment<'a>, on: Option<Message>) -> Element<'a, Message> {
    button(t(label).size(13).font(SEMIBOLD))
        .padding([8, 18])
        .on_press_maybe(on)
        .style(move |_, s| {
            let bg = match s {
                button::Status::Hovered | button::Status::Pressed => p.accent_hover,
                button::Status::Disabled => p.chip,
                button::Status::Active => p.accent,
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: if s == button::Status::Disabled { p.text_3 } else { p.on_accent },
                border: Border { radius: Radius::from(9.0), ..Default::default() },
                shadow: Shadow::default(),
                ..Default::default()
            }
        })
        .into()
}

fn secondary<'a>(p: &'static Palette, label: impl text::IntoFragment<'a>, on: Option<Message>) -> Element<'a, Message> {
    button(t(label).size(13).font(MEDIUM))
        .padding([7, 14])
        .on_press_maybe(on)
        .style(move |_, s| button::Style {
            background: Some(Background::Color(if s == button::Status::Hovered { p.hover } else { p.panel })),
            text_color: p.text,
            border: Border { color: p.border, width: 1.0, radius: Radius::from(9.0) },
            ..Default::default()
        })
        .into()
}

const BOLD: Font = Font { weight: font::Weight::Bold, ..Font::DEFAULT };
const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };
const MEDIUM: Font = Font { weight: font::Weight::Medium, ..Font::DEFAULT };

// ------------------------------------------------------------------ colours

#[derive(Clone, Copy)]
enum Tone {
    Green,
    Blue,
    Grey,
    Warn,
    Danger,
}

impl Tone {
    fn colors(self, p: &Palette) -> (Color, Color) {
        match self {
            Tone::Green => (p.green, p.green_bg),
            Tone::Blue => (p.blue, p.blue_bg),
            Tone::Grey => (p.text_2, p.chip),
            Tone::Warn => (p.warn, p.warn_bg),
            Tone::Danger => (p.danger, p.danger_bg),
        }
    }
}

struct Palette {
    chrome: Color,
    panel: Color,
    hover: Color,
    line: Color,
    chip: Color,
    border: Color,
    accent: Color,
    accent_hover: Color,
    on_accent: Color,
    text: Color,
    text_2: Color,
    text_3: Color,
    green: Color,
    green_bg: Color,
    blue: Color,
    blue_bg: Color,
    warn: Color,
    warn_bg: Color,
    danger: Color,
    danger_bg: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

const LIGHT: Palette = Palette {
    chrome: rgb(0xE7ECEC),
    panel: rgb(0xFFFFFF),
    hover: rgb(0xF6F7F8),
    line: rgb(0xF5F6F7),
    chip: rgb(0xE6E8EB),
    border: rgb(0xDDE0E4),
    accent: rgb(0x111317),
    accent_hover: rgb(0x2A2D33),
    on_accent: rgb(0xFFFFFF),
    text: rgb(0x111317),
    text_2: rgb(0x6B7280),
    text_3: rgb(0x9CA3AF),
    green: rgb(0x1F8A4C),
    green_bg: rgb(0xE3F5EA),
    blue: rgb(0x2563EB),
    blue_bg: rgb(0xEAF1FE),
    warn: rgb(0xB54708),
    warn_bg: rgb(0xFEF0DC),
    danger: rgb(0xB42318),
    danger_bg: rgb(0xFDECEA),
};

const DARK: Palette = Palette {
    chrome: rgb(0x18191C),
    panel: rgb(0x222327),
    hover: rgb(0x2A2C30),
    line: rgb(0x2A2C30),
    chip: rgb(0x34363B),
    border: rgb(0x34373C),
    accent: rgb(0xF2F3F5),
    accent_hover: rgb(0xFFFFFF),
    on_accent: rgb(0x111317),
    text: rgb(0xECEDEF),
    text_2: rgb(0xA1A5AD),
    text_3: rgb(0x6E737B),
    green: rgb(0x5FD38D),
    green_bg: rgb(0x1C3326),
    blue: rgb(0x8AB4FF),
    blue_bg: rgb(0x1D2A44),
    warn: rgb(0xF5B35C),
    warn_bg: rgb(0x3A2C17),
    danger: rgb(0xFF8A80),
    danger_bg: rgb(0x3A1F1F),
};

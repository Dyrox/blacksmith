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
use iced::widget::{Column, Space, Text, center, column, container, row, scrollable, text};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Font, Length, Padding, Size, Subscription, Task, Theme, event,
    font, system, window,
};

fn main() -> iced::Result {
    // `blacksmith a.flp b.flp …` converts without opening a window.
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if !files.is_empty() {
        for f in &files {
            let r = process(f);
            let list = r.groups.iter().map(|(n, h, c)| format!("{c}× {n} [{}]", badge(h).0)).collect::<Vec<_>>();
            println!("{} {}  {}  {}", r.kind.glyph(), r.file, r.detail, list.join(", "));
        }
        return Ok(());
    }

    iced::application(App::boot, App::update, App::view)
        .title("Blacksmith")
        .subscription(App::subscription)
        .theme(|app: &App| if app.dark { Theme::Dark } else { Theme::Light })
        .window(window::Settings {
            size: Size::new(480.0, 400.0),
            min_size: Some(Size::new(360.0, 280.0)),
            ..Default::default()
        })
        .run()
}

// ------------------------------------------------------------------ work

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Done,
    Nothing,
    Failed,
}

impl Kind {
    fn glyph(self) -> &'static str {
        match self {
            Kind::Done => "✓",
            Kind::Nothing => "–",
            Kind::Failed => "✕",
        }
    }
}

struct Outcome {
    kind: Kind,
    file: String,
    detail: String,
    /// Plugins in the project: (name, outcome, instances).
    groups: Vec<(String, How, usize)>,
}

/// Upgrades one project and writes `<name> (VST3).flp` next to it.
/// The original is never touched.
fn process(path: &Path) -> Outcome {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let fail = |detail: String| Outcome { kind: Kind::Failed, file: file.clone(), detail, groups: Vec::new() };
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("flp")) {
        return fail("不是 .flp 文件".into());
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return fail(format!("读取失败：{e}")),
    };
    let mut project = match flp::Project::parse(&bytes) {
        Ok(p) if p.to_bytes() == bytes => p,
        Ok(_) => return fail("无法无损读取这个工程".into()),
        Err(e) => return fail(format!("无法读取：{e}")),
    };
    let report = convert::upgrade(&mut project);
    let groups = report.groups();
    if report.converted() == 0 {
        let detail = if report.items.iter().any(|(_, h)| matches!(h, How::Skipped(_))) {
            "有 VST2 插件，但都无法转换"
        } else {
            "没有需要转换的 VST2 插件"
        };
        return Outcome { kind: Kind::Nothing, file, detail: detail.into(), groups };
    }
    let out = output_path(path);
    if let Err(e) = std::fs::write(&out, project.to_bytes()) {
        return Outcome { groups, ..fail(format!("保存失败：{e}")) };
    }
    let name = out.file_name().unwrap().to_string_lossy();
    Outcome { kind: Kind::Done, file, detail: format!("已另存为 {name}"), groups }
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

struct App {
    dark: bool,
    hovering: bool,
    current: Option<Outcome>,
    /// When `current` arrived, and the latest frame time (drives the reveal).
    shown_at: Instant,
    now: Instant,
}

#[derive(Debug, Clone)]
enum Message {
    Dropped(PathBuf),
    Hover(bool),
    Theme(Mode),
    Focused,
    Frame(Instant),
}

/// Reveal animation: header first, then each row a beat later.
const STAGGER: f32 = 0.07;
const FADE: f32 = 0.28;

impl App {
    fn boot() -> (Self, Task<Message>) {
        let dark = platform::system_dark().unwrap_or(false);
        let now = Instant::now();
        // Debug aid: BLACKSMITH_DROP=<file> behaves as if that file was dropped at launch.
        let current = std::env::var_os("BLACKSMITH_DROP").map(|f| process(Path::new(&f)));
        (Self { dark, hovering: false, current, shown_at: now, now }, system::theme().map(Message::Theme))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Dropped(path) => {
                self.hovering = false;
                self.current = Some(process(&path));
                self.shown_at = Instant::now();
                self.now = self.shown_at;
            }
            Message::Hover(h) => self.hovering = h,
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
        let rows = self.current.as_ref().map_or(0, |c| c.groups.len()) as f32;
        self.now.duration_since(self.shown_at) < Duration::from_secs_f32(FADE + STAGGER * (rows + 1.0) + 0.05)
    }

    /// 0 → 1 progress of the i-th element of the reveal (eased).
    fn reveal(&self, i: usize) -> f32 {
        let t = self.now.duration_since(self.shown_at).as_secs_f32() - STAGGER * i as f32;
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

    fn view(&self) -> Element<'_, Message> {
        let p = if self.dark { &DARK } else { &LIGHT };

        let body: Element<'_, Message> = match &self.current {
            None => center(
                column![
                    t("拖入 .flp 文件").size(22).font(SEMIBOLD).color(p.text),
                    t("VST2 → VST3，另存在原文件旁边").size(13).color(p.text_2),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .into(),
            Some(o) => self.result(o, p),
        };

        let (border, bg) = if self.hovering { (p.accent, p.hover) } else { (p.border, p.panel) };
        let zone = container(body).padding(22).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { color: border, width: 1.5, radius: Radius::from(14.0) },
            ..Default::default()
        });
        let chrome = p.chrome;
        container(zone)
            .padding(14)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(Background::Color(chrome)), ..Default::default() })
            .into()
    }

    fn result<'a>(&self, o: &'a Outcome, p: &'static Palette) -> Element<'a, Message> {
        let a = self.reveal(0);
        let status = match o.kind {
            Kind::Done => p.green,
            Kind::Nothing => p.text_2,
            Kind::Failed => p.danger,
        };
        let header = column![
            row![
                t(o.kind.glyph()).size(16).color(fade(status, a)),
                t(&o.file).size(16).font(SEMIBOLD).color(fade(p.text, a)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            t(&o.detail).size(12).color(fade(p.text_2, a)),
        ]
        .spacing(3);

        let mut list = Column::new().spacing(4);
        for (i, (name, how, count)) in o.groups.iter().enumerate() {
            let a = self.reveal(i + 1);
            let (label, tone) = badge(how);
            let (fg, bg) = tone.colors(p);
            let mut left = row![t(name).size(14).font(MEDIUM).color(fade(p.text, a))].spacing(6).align_y(Alignment::Center);
            if *count > 1 {
                left = left.push(t(format!("×{count}")).size(12).color(fade(p.text_2, a)));
            }
            let pill = container(t(label).size(11).font(MEDIUM).color(fade(fg, a))).padding([2, 9]).style(move |_| {
                container::Style {
                    background: Some(Background::Color(fade(bg, a))),
                    border: Border { radius: Radius::from(99.0), ..Default::default() },
                    ..Default::default()
                }
            });
            let line = fade(p.line, a);
            list = list.push(
                container(
                    row![left, Space::new().width(Length::Fill), pill].align_y(Alignment::Center).padding(Padding {
                        left: 10.0 + 12.0 * (1.0 - a), // slides in from the left
                        ..Padding::from([7, 10])
                    }),
                )
                .style(move |_| container::Style {
                    background: Some(Background::Color(line)),
                    border: Border { radius: Radius::from(8.0), ..Default::default() },
                    ..Default::default()
                }),
            );
        }
        if o.groups.is_empty() && o.kind != Kind::Failed {
            list = list.push(t("工程里没有 VST 插件").size(13).color(fade(p.text_2, self.reveal(1))));
        }

        column![
            header,
            Space::new().height(10),
            scrollable(list)
                .direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4).spacing(6)))
                .height(Length::Fill),
            t("再拖入一个文件以继续").size(11).color(p.text_3),
        ]
        .spacing(6)
        .into()
    }
}

/// Text with full shaping, so Chinese and other scripts fall back to system fonts.
fn t<'a>(s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).shaping(Shaping::Advanced)
}

fn fade(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };
const MEDIUM: Font = Font { weight: font::Weight::Medium, ..Font::DEFAULT };

// ------------------------------------------------------------------ colours

#[derive(Clone, Copy)]
enum Tone {
    Green,
    Blue,
    Grey,
    Warn,
}

impl Tone {
    fn colors(self, p: &Palette) -> (Color, Color) {
        match self {
            Tone::Green => (p.green, p.green_bg),
            Tone::Blue => (p.blue, p.blue_bg),
            Tone::Grey => (p.text_2, p.chip),
            Tone::Warn => (p.warn, p.warn_bg),
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
    border: rgb(0xD6DADE),
    accent: rgb(0x111317),
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
};

const DARK: Palette = Palette {
    chrome: rgb(0x18191C),
    panel: rgb(0x222327),
    hover: rgb(0x2A2C30),
    line: rgb(0x2A2C30),
    chip: rgb(0x34363B),
    border: rgb(0x3A3D43),
    accent: rgb(0xF2F3F5),
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
};

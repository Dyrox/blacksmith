// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod convert;
mod flp;
mod platform;

use std::path::{Path, PathBuf};

use iced::border::Radius;
use iced::theme::Mode;
use iced::widget::{Column, center, column, container, row, text};
use iced::{Alignment, Background, Border, Color, Element, Event, Length, Size, Subscription, Task, Theme, event, font, system, window};

fn main() -> iced::Result {
    // `blacksmith a.flp b.flp …` converts without opening a window.
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if !files.is_empty() {
        for f in &files {
            let r = process(f);
            println!("{} {}  {}", r.kind.glyph(), r.file, r.detail);
        }
        return Ok(());
    }

    iced::application(App::boot, App::update, App::view)
        .title("Blacksmith")
        .subscription(App::subscription)
        .theme(|app: &App| if app.dark { Theme::Dark } else { Theme::Light })
        .window(window::Settings {
            size: Size::new(460.0, 340.0),
            min_size: Some(Size::new(320.0, 240.0)),
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
}

/// Upgrades one project and writes `<name> (VST3).flp` next to it.
/// The original is never touched.
fn process(path: &Path) -> Outcome {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let result = (|| -> Result<Option<(String, PathBuf)>, String> {
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("flp")) {
            return Err("not an .flp file".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut project = flp::Project::parse(&bytes)?;
        if project.to_bytes() != bytes {
            return Err("can't read this project losslessly".into());
        }
        let report = convert::upgrade(&mut project)?;
        if report.total() == 0 {
            return Ok(None);
        }
        let out = output_path(path);
        std::fs::write(&out, project.to_bytes()).map_err(|e| e.to_string())?;
        let what = report.upgraded.iter().map(|(name, n)| format!("{n}× {name}")).collect::<Vec<_>>().join(", ");
        Ok(Some((what, out)))
    })();
    match result {
        Ok(Some((what, out))) => Outcome {
            kind: Kind::Done,
            file,
            detail: format!("{what} → {}", out.file_name().unwrap().to_string_lossy()),
        },
        Ok(None) => Outcome { kind: Kind::Nothing, file, detail: "no VST2 plugins to upgrade".into() },
        Err(e) => Outcome { kind: Kind::Failed, file, detail: e },
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

// ------------------------------------------------------------------ app

struct App {
    dark: bool,
    hovering: bool,
    results: Vec<Outcome>,
}

#[derive(Debug, Clone)]
enum Message {
    Dropped(PathBuf),
    Hover(bool),
    Theme(Mode),
    Focused,
}

const MAX_RESULTS: usize = 6;

impl App {
    fn boot() -> (Self, Task<Message>) {
        let dark = platform::system_dark().unwrap_or(false);
        (Self { dark, hovering: false, results: Vec::new() }, system::theme().map(Message::Theme))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Dropped(path) => {
                self.hovering = false;
                self.results.insert(0, process(&path));
                self.results.truncate(MAX_RESULTS);
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
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            event::listen_with(|event, _, _| match event {
                Event::Window(window::Event::FileDropped(p)) => Some(Message::Dropped(p)),
                Event::Window(window::Event::FileHovered(_)) => Some(Message::Hover(true)),
                Event::Window(window::Event::FilesHoveredLeft) => Some(Message::Hover(false)),
                Event::Window(window::Event::Focused) => Some(Message::Focused),
                _ => None,
            }),
            system::theme_changes().map(Message::Theme),
        ])
    }

    fn view(&self) -> Element<'_, Message> {
        let p = if self.dark { &DARK } else { &LIGHT };
        let bold = iced::Font { weight: font::Weight::Semibold, ..iced::Font::DEFAULT };

        let mut body = Column::new().spacing(6).align_x(Alignment::Center).push(
            column![
                text("Drop .flp files").size(22).font(bold).color(p.text),
                text("VST2 → VST3 · saved next to the original").size(13).color(p.text_2),
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        );
        if !self.results.is_empty() {
            body = body.push(iced::widget::Space::new().height(14));
        }
        for r in &self.results {
            let color = match r.kind {
                Kind::Done => p.green,
                Kind::Nothing => p.text_2,
                Kind::Failed => p.danger,
            };
            body = body.push(
                row![
                    text(r.kind.glyph()).size(13).color(color),
                    text(&r.file).size(13).font(bold).color(p.text),
                    text(&r.detail).size(13).color(p.text_2),
                ]
                .spacing(8),
            );
        }

        let (border, bg) = if self.hovering { (p.accent, p.hover) } else { (p.border, p.panel) };
        let zone = container(center(body).padding(20)).width(Length::Fill).height(Length::Fill).style(move |_| {
            container::Style {
                background: Some(Background::Color(bg)),
                border: Border { color: border, width: 1.5, radius: Radius::from(14.0) },
                ..Default::default()
            }
        });
        let chrome = p.chrome;
        container(zone)
            .padding(14)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(Background::Color(chrome)), ..Default::default() })
            .into()
    }
}

// ------------------------------------------------------------------ colours

struct Palette {
    chrome: Color,
    panel: Color,
    hover: Color,
    border: Color,
    accent: Color,
    text: Color,
    text_2: Color,
    green: Color,
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
    border: rgb(0xD6DADE),
    accent: rgb(0x111317),
    text: rgb(0x111317),
    text_2: rgb(0x6B7280),
    green: rgb(0x1F8A4C),
    danger: rgb(0xB42318),
};

const DARK: Palette = Palette {
    chrome: rgb(0x18191C),
    panel: rgb(0x222327),
    hover: rgb(0x2A2C30),
    border: rgb(0x3A3D43),
    accent: rgb(0xF2F3F5),
    text: rgb(0xECEDEF),
    text_2: rgb(0xA1A5AD),
    green: rgb(0x5FD38D),
    danger: rgb(0xFF8A80),
};

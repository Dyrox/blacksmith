// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod convert;
mod flp;
mod plugins;
mod style;
mod vst3;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use convert::How;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Column, Space, button, center, column, container, row, scrollable, stack};
use iced::{Alignment, Element, Event, Length, Padding, Size, Subscription, Task, Theme, event, window};
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
        .theme(|_: &App| Theme::Dark)
        .window(window::Settings {
            size: Size::new(860.0, 600.0),
            min_size: Some(Size::new(680.0, 480.0)),
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
        How::Skipped(why) => (format!("跳过 · {why}"), Tone::Warn),
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
    page: Page,
    hovering: bool,
    items: Vec<Item>,
    now: Instant,
    /// Animation clocks: launch, last page switch, last drag-hover change,
    /// last change to what the big button does.
    boot_at: Instant,
    page_at: Instant,
    hover_at: Instant,
    cta_at: Instant,
    /// The launch animation starts at the first rendered frame, not at boot,
    /// so a slow GPU start doesn't eat it.
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
    Frame(Instant),
}

/// Reveal animation: card header first, then each plugin row a beat later.
const STAGGER: f32 = 0.06;
const FADE: f32 = 0.30;
const SLIDE: f32 = 0.55;

fn ease_out_cubic(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// Overshoots past 1 and settles back: the "pop".
fn ease_out_back(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    const C1: f32 = 1.9;
    const C3: f32 = C1 + 1.0;
    1.0 + C3 * (x - 1.0).powi(3) + C1 * (x - 1.0).powi(2)
}

impl App {
    fn boot() -> Self {
        let now = Instant::now();
        let mut app = Self {
            page: Page::Convert,
            hovering: false,
            items: Vec::new(),
            now,
            boot_at: now,
            page_at: now,
            // As if the last hover ended long ago, so nothing swells at launch.
            hover_at: now.checked_sub(Duration::from_secs(10)).unwrap_or(now),
            cta_at: now,
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
        app
    }

    fn add(&mut self, path: PathBuf) {
        let item = Item { state: run(&path, false), path, shown_at: Instant::now() };
        // Dropping the same file again re-analyses it in place.
        match self.items.iter_mut().find(|i| i.path == item.path) {
            Some(existing) => *existing = item,
            None => self.items.push(item),
        }
        self.go(Page::Convert);
        self.cta_at = Instant::now();
        self.now = Instant::now();
    }

    fn go(&mut self, page: Page) {
        if self.page != page {
            self.page = page;
            self.page_at = Instant::now();
        }
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
                self.cta_at = now;
                self.now = now;
            }
            Message::Clear => {
                self.items.clear();
                self.boot_at = Instant::now(); // replay the welcome pop
                self.cta_at = self.boot_at;
                self.now = self.boot_at;
            }
            Message::Remove(i) => {
                if i < self.items.len() {
                    self.items.remove(i);
                }
            }
            Message::Reveal(path) => reveal_in_folder(&path),
            Message::Frame(now) => {
                if !self.started {
                    self.started = true;
                    self.boot_at = now;
                    self.cta_at = now;
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
            || self.since(self.boot_at) < 1.0
            || self.since(self.page_at) < 0.7
            || self.since(self.hover_at) < 0.7
            || self.since(self.cta_at) < 0.7
    }

    /// The n-th element of an item's reveal: (opacity, springy slide progress).
    fn reveal(&self, item: &Item, n: usize) -> (f32, f32) {
        let t = self.since(item.shown_at) - STAGGER * n as f32;
        (ease_out_cubic(t / FADE), ease_out_back(t / SLIDE))
    }

    /// Scale of the welcome orb: pops in at launch, swells while a file hovers.
    fn orb_scale(&self, delay: f32) -> f32 {
        let pop = 0.55 + 0.45 * ease_out_back((self.since(self.boot_at) - delay) / 0.7);
        let k = ease_out_back(self.since(self.hover_at) / 0.55);
        let swell = if self.hovering { 1.0 + 0.14 * k } else { 1.14 - 0.14 * k };
        pop * swell
    }

    /// Scale of the big round button: pops whenever what it does changes.
    fn cta_scale(&self) -> f32 {
        0.7 + 0.3 * ease_out_back(self.since(self.cta_at) / 0.55)
    }

    /// Extra left offset of the page content while it slides in.
    fn page_offset(&self) -> f32 {
        36.0 * (1.0 - ease_out_back(self.since(self.page_at) / 0.6))
    }

    fn subscription(&self) -> Subscription<Message> {
        let events = event::listen_with(|event, _, _| match event {
            Event::Window(window::Event::FileDropped(p)) => Some(Message::Dropped(p)),
            Event::Window(window::Event::FileHovered(_)) => Some(Message::Hover(true)),
            Event::Window(window::Event::FilesHoveredLeft) => Some(Message::Hover(false)),
            _ => None,
        });
        let frames = if self.animating() { window::frames().map(Message::Frame) } else { Subscription::none() };
        Subscription::batch([events, frames])
    }

    // -------------------------------------------------------------- views

    fn view(&self) -> Element<'_, Message> {
        let main = match self.page {
            Page::Convert if self.items.is_empty() => self.welcome(),
            Page::Convert => self.projects(),
            Page::Supported => self.supported(),
        };
        let backdrop = match self.page {
            Page::Convert => FORGE,
            Page::Supported => TIDE,
        };
        let hovering = self.hovering;
        let main = container(container(main).padding(Padding { left: self.page_offset().max(-6.0), ..Padding::ZERO }))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| backdrop_style(backdrop, hovering));
        row![self.sidebar(), main].into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let nav = |label: &'static str, glyph: &'static str, page: Page| {
            let active = self.page == page;
            button(
                row![
                    container(t(glyph).size(13).color(WHITE)).width(26).height(26).center_x(26).center_y(26).style(
                        move |_| icon_tile(if page == Page::Convert { FORGE } else { TIDE })
                    ),
                    t(label).size(14).font(MEDIUM).color(if active { WHITE } else { TEXT_2 }),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([7, 8])
            .on_press(Message::Nav(page))
            .style(move |_, s| nav_button(active, s))
        };
        let logo = row![
            container(t("B").size(16).font(BOLD).color(WHITE)).width(30).height(30).center_x(30).center_y(30).style(|_| logo_tile()),
            column![t("Blacksmith").size(15).font(BOLD).color(WHITE), t("VST2 → VST3").size(11).color(TEXT_3)],
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        container(
            column![
                logo,
                Space::new().height(22),
                nav("转换工程", "⇄", Page::Convert),
                nav("官方支持", "✓", Page::Supported),
                Space::new().height(Length::Fill),
                t("FL Studio · .flp").size(11).color(TEXT_3),
            ]
            .spacing(4),
        )
        .width(210)
        .height(Length::Fill)
        .padding(Padding { top: 22.0, right: 14.0, bottom: 18.0, left: 14.0 })
        .style(|_| sidebar())
        .into()
    }

    /// Empty state: a big glowing drop target and the round action button.
    fn welcome(&self) -> Element<'_, Message> {
        let h = self.hovering;
        let ring = |size: f32, alpha: f32, delay: f32| {
            let size = size * self.orb_scale(delay);
            container(Space::new()).width(size).height(size).style(move |_| ring(size, alpha * if h { 1.8 } else { 1.0 }))
        };
        let d = 132.0 * self.orb_scale(0.0);
        let core = container(t("↓").size(54.0 * d / 132.0).font(BOLD).color(WHITE))
            .width(d)
            .height(d)
            .center_x(d)
            .center_y(d)
            .style(move |_| orb(d, h));
        let hero = stack![
            center(ring(300.0, 0.05, 0.16)),
            center(ring(236.0, 0.08, 0.10)),
            center(ring(178.0, 0.12, 0.05)),
            center(core),
        ]
        .width(360)
        .height(360);

        column![
            Space::new().height(28),
            t("给你的工程换上 VST3").size(30).font(BOLD).color(WHITE),
            t(if h { "松手即可分析" } else { "把 FL Studio 工程（.flp）拖进窗口，先看插件清单，再决定转换" }).size(14).color(TEXT_2),
            Space::new().height(Length::Fill),
            hero,
            Space::new().height(Length::Fill),
            big_button("选择", Some(Message::Browse), self.cta_scale()),
            t("原文件不会被改动，转换结果另存在旁边").size(12).color(TEXT_3),
            Space::new().height(26),
        ]
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .into()
    }

    fn projects(&self) -> Element<'_, Message> {
        let ready = self.ready();
        let done = self.items.iter().filter(|i| matches!(i.state, State::Done { .. })).count();
        let (title, subtitle) = match (ready, done) {
            (0, 0) => ("没有可转换的插件".to_string(), "这些工程里没有能升级的 VST2 插件".to_string()),
            (0, d) => ("转换完成".to_string(), format!("{d} 个工程已另存为 VST3 版本")),
            (r, _) => (format!("{r} 个插件可以升级"), format!("{} 个工程 · 确认清单后点「转换」", self.items.len())),
        };

        let mut cards = Column::new().spacing(12).padding(Padding { right: 12.0, ..Padding::ZERO });
        for (i, item) in self.items.iter().enumerate() {
            cards = cards.push(self.card(i, item));
        }
        cards = cards.push(container(t("拖入更多 .flp 文件").size(12).color(TEXT_3)).center_x(Length::Fill).padding(6));

        let action = if ready > 0 {
            big_button("转换", Some(Message::Convert), self.cta_scale())
        } else {
            big_button("完成", None, self.cta_scale())
        };
        let footer = row![
            container(link_button("清空", Message::Clear)).width(Length::Fill),
            action,
            container(link_button("添加文件…", Message::Browse)).width(Length::Fill).align_right(Length::Fill),
        ]
        .align_y(Alignment::Center);

        column![
            column![t(title).size(26).font(BOLD).color(WHITE), t(subtitle).size(13).color(TEXT_2)].spacing(4),
            scrollable(cards)
                .direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4)))
                .height(Length::Fill),
            footer,
        ]
        .spacing(16)
        .padding(Padding { top: 30.0, right: 28.0, bottom: 22.0, left: 32.0 })
        .into()
    }

    fn card<'a>(&self, index: usize, item: &'a Item) -> Element<'a, Message> {
        let (a, slide) = self.reveal(item, 0);
        let file = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = item.path.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let (chip, tone) = match &item.state {
            State::Ready { .. } => ("待转换", Tone::Blue),
            State::Done { .. } => ("已转换", Tone::Green),
            State::Nothing { .. } => ("无需转换", Tone::Grey),
            State::Failed(_) => ("出错", Tone::Danger),
        };
        let remove = button(t("✕").size(12).color(fade(TEXT_3, a)))
            .padding([2, 6])
            .on_press(Message::Remove(index))
            .style(|_, s| ghost_button(s));
        let headline = match item.state {
            State::Done { .. } => GREEN,
            State::Failed(_) => DANGER,
            _ => TEXT_2,
        };
        let mut body = column![
            row![
                container(t("FL").size(12).font(BOLD).color(WHITE)).width(34).height(34).center_x(34).center_y(34).style(
                    move |_| fade_tile(icon_tile(FORGE), a)
                ),
                column![
                    t(file).size(15).font(SEMIBOLD).color(fade(WHITE, a)),
                    t(dir).size(11).color(fade(TEXT_3, a)),
                ]
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
            let mut list = Column::new().spacing(5);
            for (i, (name, how, count)) in groups.iter().enumerate() {
                let (a, k) = self.reveal(item, i + 1);
                let (label, tone) = badge(how);
                let mut left = row![t(name).size(13).font(MEDIUM).color(fade(WHITE, a))].spacing(6).align_y(Alignment::Center);
                if *count > 1 {
                    left = left.push(t(format!("×{count}")).size(12).color(fade(TEXT_3, a)));
                }
                list = list.push(
                    container(
                        row![left, Space::new().width(Length::Fill), pill(label, tone, a)]
                            .align_y(Alignment::Center)
                            .padding(Padding { left: 12.0 + 22.0 * (1.0 - k), ..Padding::from([7, 12]) }),
                    )
                    .style(move |_| row_style(a)),
                );
            }
            body = body.push(list);
        } else if !matches!(item.state, State::Failed(_)) {
            body = body.push(t("工程里没有 VST 插件").size(12).color(fade(TEXT_3, a)));
        }
        if let State::Done { out, .. } = &item.state {
            body = body.push(row![Space::new().width(Length::Fill), glass_button("在文件夹中显示", Message::Reveal(out.clone()))]);
        }
        // Card drops in from slightly below with a springy settle.
        let lift = 18.0 * (1.0 - slide);
        container(container(body).padding(16).width(Length::Fill).style(move |_| glass(a)))
            .padding(Padding { top: lift.max(0.0), bottom: (-lift).max(0.0), ..Padding::ZERO })
            .into()
    }

    fn supported(&self) -> Element<'_, Message> {
        let mut rules = Column::new().spacing(10);
        for r in plugins::RULES {
            rules = rules.push(
                container(
                    row![
                        container(t(&r.name[..1]).size(16).font(BOLD).color(WHITE)).width(40).height(40).center_x(40).center_y(40).style(
                            |_| icon_tile(TIDE)
                        ),
                        column![
                            row![t(r.name).size(15).font(SEMIBOLD).color(WHITE), t(r.vendor).size(12).color(TEXT_3)]
                                .spacing(8)
                                .align_y(Alignment::Center),
                            t(r.note).size(12).color(TEXT_2),
                        ]
                        .spacing(3)
                        .width(Length::Fill),
                        pill("官方支持", Tone::Green, 1.0),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding(14)
                .style(|_| glass(1.0)),
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
                    t(text).size(12).color(TEXT_2).width(Length::Fill),
                ]
                .align_y(Alignment::Center),
            );
        }

        scrollable(
            column![
                column![
                    t("官方支持的插件").size(26).font(BOLD).color(WHITE),
                    t("这些插件有专门的转换规则，音色和设置会原样迁移").size(13).color(TEXT_2),
                ]
                .spacing(4),
                rules,
                Space::new().height(8),
                t("标签说明").size(15).font(SEMIBOLD).color(WHITE),
                container(explain).padding(16).style(|_| glass(1.0)),
                t("想加新的官方支持：提供插件的 VST2 和 VST3，以及一个两种格式都加载了同一个预设的工程。").size(12).color(TEXT_3),
            ]
            .spacing(14)
            .padding(Padding { top: 30.0, right: 40.0, bottom: 24.0, left: 32.0 }),
        )
        .direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4)))
        .height(Length::Fill)
        .into()
    }
}

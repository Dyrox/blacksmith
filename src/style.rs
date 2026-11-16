//! Look: neutral, near-monochrome surfaces that follow the system's light/dark
//! setting; the primary action is ink on paper (black in light mode, white in
//! dark); colour only marks status. Hairline borders, flat cards, generous hit
//! targets. Read the palette anywhere with `p()`.

use std::sync::atomic::{AtomicBool, Ordering};

use iced::border::Radius;
use iced::widget::text::Shaping;
use iced::widget::{Row, Text, button, container, row, rule, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Font, Shadow, font};

use crate::Message;

pub const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

pub struct Palette {
    /// Window background behind the content.
    pub bg: Color,
    pub sidebar: Color,
    pub panel: Color,
    pub row: Color,
    pub hover: Color,
    pub selected: Color,
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_2: Color,
    pub text_3: Color,
    pub accent_soft: Color,
    pub green: Color,
    pub blue: Color,
    pub amber: Color,
    pub red: Color,
    pub grey: Color,
}

pub const LIGHT: Palette = Palette {
    bg: rgb(0xF6F7F9),
    sidebar: rgb(0xECEEF1),
    panel: rgb(0xFFFFFF),
    row: rgb(0xF4F5F7),
    hover: rgb(0xE4E7EB),
    selected: rgb(0xDCE0E6),
    border: rgb(0xE3E6EA),
    border_strong: rgb(0xCDD2D9),
    text: rgb(0x15171A),
    text_2: rgb(0x5F6672),
    text_3: rgb(0x9097A1),
    accent_soft: rgb(0xE8EFFE),
    green: rgb(0x1A7F45),
    blue: rgb(0x2563EB),
    amber: rgb(0xB45309),
    red: rgb(0xB42318),
    grey: rgb(0x5F6672),
};

pub const DARK: Palette = Palette {
    bg: rgb(0x1B1C1F),
    sidebar: rgb(0x151618),
    panel: rgb(0x242528),
    row: rgb(0x2B2C30),
    hover: rgb(0x2E3034),
    selected: rgb(0x33363B),
    border: rgb(0x2F3135),
    border_strong: rgb(0x43464C),
    text: rgb(0xEDEEF0),
    text_2: rgb(0xA4A9B1),
    text_3: rgb(0x70757D),
    accent_soft: rgb(0x1E2B45),
    green: rgb(0x5FD38D),
    blue: rgb(0x8AB4FF),
    amber: rgb(0xF5B35C),
    red: rgb(0xFF8A80),
    grey: rgb(0xA4A9B1),
};

static IS_DARK: AtomicBool = AtomicBool::new(false);

pub fn set_dark(dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
}

pub fn p() -> &'static Palette {
    if IS_DARK.load(Ordering::Relaxed) { &DARK } else { &LIGHT }
}

pub fn fade(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

/// Text with full shaping, so Chinese and other scripts fall back to system fonts.
pub fn t<'a>(s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).shaping(Shaping::Advanced)
}

pub const BOLD: Font = Font { weight: font::Weight::Bold, ..Font::DEFAULT };
pub const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };
pub const MEDIUM: Font = Font { weight: font::Weight::Medium, ..Font::DEFAULT };

fn radius(r: f32) -> Radius {
    Radius::from(r)
}

// ------------------------------------------------------------------ surfaces

pub fn background() -> container::Style {
    container::Style { background: Some(Background::Color(p().bg)), ..Default::default() }
}

pub fn sidebar() -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(p.sidebar)),
        border: Border { color: p.border, width: 1.0, radius: radius(0.0) },
        ..Default::default()
    }
}

/// A flat card with a hairline border; `a` fades it in.
pub fn card(a: f32) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(fade(p.panel, a))),
        border: Border { color: fade(p.border, a), width: 1.0, radius: radius(10.0) },
        ..Default::default()
    }
}

/// A quiet grey box for notes and the step number circles.
pub fn note() -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(p.row)),
        border: Border { color: p.border, width: 1.0, radius: radius(8.0) },
        ..Default::default()
    }
}

pub fn step_number() -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(p.row)),
        border: Border { color: p.border, width: 1.0, radius: radius(99.0) },
        ..Default::default()
    }
}

/// The bar above each page's content.
pub fn top_bar() -> container::Style {
    container::Style { background: Some(Background::Color(p().bg)), ..Default::default() }
}

/// A hairline separator.
pub fn hairline<'a>() -> Element<'a, Message> {
    rule::horizontal(1)
        .style(|_| rule::Style { color: p().border, radius: radius(0.0), fill_mode: rule::FillMode::Full, snap: true })
        .into()
}

/// Table header cell text.
pub fn th<'a>(s: impl text::IntoFragment<'a>) -> Text<'a> {
    t(s).size(11).font(MEDIUM).color(p().text_3)
}



/// The drop target; highlighted while a file is dragged over the window.
pub fn drop_zone(hovering: bool) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(if hovering { p.row } else { p.panel })),
        border: Border {
            color: if hovering { p.text_2 } else { p.border_strong },
            width: 1.0,
            radius: radius(10.0),
        },
        ..Default::default()
    }
}

pub fn icon_circle(size: f32, hovering: bool) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(if hovering { p.text } else { p.row })),
        border: Border { color: p.border, width: if hovering { 0.0 } else { 1.0 }, radius: radius(size / 2.0) },
        ..Default::default()
    }
}



pub fn count_badge(active: bool) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(if active { p.panel } else { p.selected })),
        border: Border { radius: radius(99.0), ..Default::default() },
        ..Default::default()
    }
}

// ------------------------------------------------------------------ pills

#[derive(Clone, Copy)]
pub enum Tone {
    Green,
    Blue,
    Grey,
    Amber,
    Red,
}

impl Tone {
    fn color(self) -> Color {
        let p = p();
        match self {
            Tone::Green => p.green,
            Tone::Blue => p.blue,
            Tone::Grey => p.grey,
            Tone::Amber => p.amber,
            Tone::Red => p.red,
        }
    }
}

impl Tone {
    /// The mark in front of a status: ✓ done or fine, → will change, ⚠ needs
    /// attention, – nothing to do, ✕ failed.
    pub fn glyph(self) -> &'static str {
        match self {
            Tone::Green => "✓",
            Tone::Blue => "→",
            Tone::Amber => "⚠",
            Tone::Grey => "–",
            Tone::Red => "✕",
        }
    }
}

/// A status as livesaver-style mark + words: colour on the mark only.
pub fn status<'a>(label: impl text::IntoFragment<'a>, tone: Tone, a: f32) -> Row<'a, Message> {
    let p = p();
    row![
        t(tone.glyph()).size(12).font(SEMIBOLD).color(fade(tone.color(), a)),
        t(label).size(12).color(fade(if matches!(tone, Tone::Grey) { p.text_3 } else { p.text }, a)),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
}


// ------------------------------------------------------------------ buttons

/// Primary action: ink on paper (near-black in light mode, near-white in dark).
pub fn primary(s: button::Status) -> button::Style {
    let p = p();
    let bg = match s {
        button::Status::Hovered | button::Status::Pressed => fade(p.text, 0.82),
        button::Status::Disabled => p.selected,
        button::Status::Active => p.text,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: if s == button::Status::Disabled { p.text_3 } else { p.panel },
        border: Border { radius: radius(8.0), ..Default::default() },
        shadow: Shadow::default(),
        ..Default::default()
    }
}

/// A tab in a segmented row: the active one is a raised panel.
pub fn tab(active: bool, s: button::Status) -> button::Style {
    let p = p();
    button::Style {
        background: if active {
            Some(Background::Color(p.panel))
        } else {
            (s == button::Status::Hovered).then_some(Background::Color(p.hover))
        },
        text_color: if active { p.text } else { p.text_2 },
        border: Border { color: if active { p.border } else { Color::TRANSPARENT }, width: 1.0, radius: radius(7.0) },
        ..Default::default()
    }
}

/// The track the tabs sit in.
pub fn tabs() -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(p.row)),
        border: Border { color: p.border, width: 1.0, radius: radius(9.0) },
        ..Default::default()
    }
}

pub fn secondary(s: button::Status) -> button::Style {
    let p = p();
    button::Style {
        background: Some(Background::Color(if s == button::Status::Hovered { p.hover } else { p.panel })),
        text_color: if s == button::Status::Disabled { p.text_3 } else { p.text },
        border: Border { color: p.border_strong, width: 1.0, radius: radius(8.0) },
        ..Default::default()
    }
}

pub fn ghost(s: button::Status) -> button::Style {
    let p = p();
    button::Style {
        background: (s == button::Status::Hovered).then_some(Background::Color(p.hover)),
        text_color: p.text_2,
        border: Border { radius: radius(8.0), ..Default::default() },
        ..Default::default()
    }
}

pub fn nav(active: bool, s: button::Status) -> button::Style {
    let p = p();
    let bg = if active {
        Some(Background::Color(p.selected))
    } else if s == button::Status::Hovered {
        Some(Background::Color(p.hover))
    } else {
        None
    };
    button::Style { background: bg, border: Border { radius: radius(8.0), ..Default::default() }, ..Default::default() }
}

// ------------------------------------------------------------------ inputs

/// A tick box: ink when ticked, like the primary button.
pub fn tick(s: iced::widget::checkbox::Status) -> iced::widget::checkbox::Style {
    use iced::widget::checkbox::Status;
    let p = p();
    let (checked, hovered, disabled) = match s {
        Status::Active { is_checked } => (is_checked, false, false),
        Status::Hovered { is_checked } => (is_checked, true, false),
        Status::Disabled { is_checked } => (is_checked, false, true),
    };
    iced::widget::checkbox::Style {
        background: Background::Color(if checked && !disabled { p.text } else if hovered { p.hover } else { p.panel }),
        icon_color: p.panel,
        border: Border { color: if checked { p.text } else { p.border_strong }, width: 1.0, radius: radius(4.0) },
        text_color: None,
    }
}

/// The dimmed backdrop behind a dialog.
pub fn backdrop() -> container::Style {
    container::Style { background: Some(Background::Color(Color { a: 0.35, ..Color::BLACK })), ..Default::default() }
}

/// A dialog panel.
pub fn dialog() -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(p.panel)),
        border: Border { color: p.border_strong, width: 1.0, radius: radius(12.0) },
        shadow: Shadow { color: Color { a: 0.25, ..Color::BLACK }, offset: iced::Vector::new(0.0, 8.0), blur_radius: 24.0 },
        ..Default::default()
    }
}

pub fn search_field(s: text_input::Status) -> text_input::Style {
    let p = p();
    let focused = matches!(s, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(p.panel),
        border: Border {
            color: if focused { p.text_2 } else { p.border_strong },
            width: 1.0,
            radius: radius(8.0),
        },
        icon: p.text_3,
        placeholder: p.text_3,
        value: p.text,
        selection: p.accent_soft,
    }
}

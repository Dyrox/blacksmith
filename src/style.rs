//! Look: neutral surfaces that follow the system's light/dark setting, one
//! calm blue accent, generous hit targets. Read the palette anywhere with `p()`.

use std::sync::atomic::{AtomicBool, Ordering};

use iced::border::Radius;
use iced::widget::text::Shaping;
use iced::widget::{Text, button, container, text};
use iced::{Background, Border, Color, Element, Font, Shadow, Vector, font};

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
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_soft: Color,
    pub on_accent: Color,
    pub green: Color,
    pub blue: Color,
    pub amber: Color,
    pub red: Color,
    pub grey: Color,
    pub shadow: f32,
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
    accent: rgb(0x2563EB),
    accent_hover: rgb(0x1D4FD8),
    accent_soft: rgb(0xE8EFFE),
    on_accent: rgb(0xFFFFFF),
    green: rgb(0x1A7F45),
    blue: rgb(0x2563EB),
    amber: rgb(0xB45309),
    red: rgb(0xB42318),
    grey: rgb(0x5F6672),
    shadow: 0.06,
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
    accent: rgb(0x4C86F7),
    accent_hover: rgb(0x6A9BFA),
    accent_soft: rgb(0x1E2B45),
    on_accent: rgb(0xFFFFFF),
    green: rgb(0x5FD38D),
    blue: rgb(0x8AB4FF),
    amber: rgb(0xF5B35C),
    red: rgb(0xFF8A80),
    grey: rgb(0xA4A9B1),
    shadow: 0.3,
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

/// A card; `a` fades it in.
pub fn card(a: f32) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(fade(p.panel, a))),
        border: Border { color: fade(p.border, a), width: 1.0, radius: radius(12.0) },
        shadow: Shadow { color: fade(Color::BLACK, p.shadow * a), offset: Vector::new(0.0, 2.0), blur_radius: 8.0 },
        ..Default::default()
    }
}

pub fn row_style(a: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(fade(p().row, a))),
        border: Border { radius: radius(8.0), ..Default::default() },
        ..Default::default()
    }
}

/// The drop target; highlighted while a file is dragged over the window.
pub fn drop_zone(hovering: bool) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(if hovering { p.accent_soft } else { p.panel })),
        border: Border {
            color: if hovering { p.accent } else { p.border_strong },
            width: if hovering { 2.0 } else { 1.0 },
            radius: radius(16.0),
        },
        ..Default::default()
    }
}

pub fn icon_circle(size: f32, hovering: bool) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(if hovering { p.accent } else { p.accent_soft })),
        border: Border { radius: radius(size / 2.0), ..Default::default() },
        ..Default::default()
    }
}

pub fn tile(a: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(fade(p().accent_soft, a))),
        border: Border { radius: radius(8.0), ..Default::default() },
        ..Default::default()
    }
}

pub fn logo() -> container::Style {
    container::Style {
        background: Some(Background::Color(p().text)),
        border: Border { radius: radius(8.0), ..Default::default() },
        ..Default::default()
    }
}

pub fn count_badge(active: bool) -> container::Style {
    let p = p();
    container::Style {
        background: Some(Background::Color(if active { p.accent } else { p.selected })),
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

pub fn pill<'a>(label: impl text::IntoFragment<'a>, tone: Tone, a: f32) -> Element<'a, Message> {
    let c = tone.color();
    container(t(label).size(11).font(MEDIUM).color(fade(c, a)))
        .padding([3, 10])
        .style(move |_| container::Style {
            background: Some(Background::Color(fade(c, 0.12 * a))),
            border: Border { radius: radius(99.0), ..Default::default() },
            ..Default::default()
        })
        .into()
}

// ------------------------------------------------------------------ buttons

/// Primary action: accent fill.
pub fn primary(s: button::Status) -> button::Style {
    let p = p();
    let bg = match s {
        button::Status::Hovered | button::Status::Pressed => p.accent_hover,
        button::Status::Disabled => p.selected,
        button::Status::Active => p.accent,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: if s == button::Status::Disabled { p.text_3 } else { p.on_accent },
        border: Border { radius: radius(10.0), ..Default::default() },
        shadow: Shadow::default(),
        ..Default::default()
    }
}

pub fn secondary(s: button::Status) -> button::Style {
    let p = p();
    button::Style {
        background: Some(Background::Color(if s == button::Status::Hovered { p.hover } else { p.panel })),
        text_color: if s == button::Status::Disabled { p.text_3 } else { p.text },
        border: Border { color: p.border_strong, width: 1.0, radius: radius(10.0) },
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

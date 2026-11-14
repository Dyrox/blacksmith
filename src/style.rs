//! Look: dark, saturated gradient backdrops, translucent "glass" panels and a
//! big glowing round action button. One fixed dark theme.

use iced::border::Radius;
use iced::gradient::Linear;
use iced::widget::text::Shaping;
use iced::widget::{Text, button, container, text};
use iced::{Background, Border, Color, Element, Font, Gradient, Length, Radians, Shadow, Vector, font};

use crate::Message;

pub const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

const fn white(a: f32) -> Color {
    Color { r: 1.0, g: 1.0, b: 1.0, a }
}

pub const WHITE: Color = white(1.0);
pub const TEXT_2: Color = white(0.70);
pub const TEXT_3: Color = white(0.45);
pub const GREEN: Color = rgb(0x6EE7A0);
pub const BLUE: Color = rgb(0x9CC2FF);
pub const AMBER: Color = rgb(0xFFC56B);
pub const DANGER: Color = rgb(0xFF8A80);

/// Three-stop backdrop gradients.
pub type Backdrop = [u32; 3];
/// Converting: violet → plum → ember.
pub const FORGE: Backdrop = [0x1E0B3B, 0x4A1452, 0x7A2A3A];
/// Info pages: deep teal → navy → indigo.
pub const TIDE: Backdrop = [0x062A3A, 0x0E2F5E, 0x241656];
/// The action button: pink → orange.
const EMBER: [u32; 2] = [0xFF4F8B, 0xFF9A3D];

fn linear(deg: f32, stops: &[u32]) -> Background {
    let mut g = Linear::new(Radians(deg.to_radians()));
    let n = (stops.len() - 1).max(1) as f32;
    for (i, c) in stops.iter().enumerate() {
        g = g.add_stop(i as f32 / n, rgb(*c));
    }
    Background::Gradient(Gradient::Linear(g))
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

pub fn backdrop_style(b: Backdrop, hovering: bool) -> container::Style {
    container::Style {
        background: Some(linear(135.0, &b)),
        border: Border { color: if hovering { white(0.55) } else { Color::TRANSPARENT }, width: 2.0, radius: radius(0.0) },
        ..Default::default()
    }
}

pub fn sidebar() -> container::Style {
    container::Style {
        background: Some(Background::Color(rgb(0x120A1F))),
        border: Border { color: white(0.06), width: 1.0, radius: radius(0.0) },
        ..Default::default()
    }
}

/// Translucent panel; `a` fades it in.
pub fn glass(a: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(white(0.07 * a))),
        border: Border { color: white(0.12 * a), width: 1.0, radius: radius(16.0) },
        shadow: Shadow { color: fade(Color::BLACK, 0.18 * a), offset: Vector::new(0.0, 6.0), blur_radius: 18.0 },
        ..Default::default()
    }
}

pub fn row_style(a: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(white(0.05 * a))),
        border: Border { radius: radius(10.0), ..Default::default() },
        ..Default::default()
    }
}

pub fn icon_tile(b: Backdrop) -> container::Style {
    let lift = |c: u32| {
        let c = rgb(c);
        let up = |x: f32| (x * 1.9 + 0.12).min(1.0);
        Color::from_rgb(up(c.r), up(c.g), up(c.b))
    };
    let mut g = Linear::new(Radians(135f32.to_radians()));
    g = g.add_stop(0.0, lift(b[1])).add_stop(1.0, lift(b[2]));
    container::Style {
        background: Some(Background::Gradient(Gradient::Linear(g))),
        border: Border { color: white(0.18), width: 1.0, radius: radius(9.0) },
        ..Default::default()
    }
}

pub fn fade_tile(mut s: container::Style, a: f32) -> container::Style {
    s.border.color = fade(s.border.color, a);
    if let Some(Background::Gradient(Gradient::Linear(g))) = &mut s.background {
        for stop in g.stops.iter_mut().flatten() {
            stop.color = fade(stop.color, a);
        }
    }
    s
}

pub fn logo_tile() -> container::Style {
    container::Style {
        background: Some(linear(135.0, &EMBER)),
        border: Border { radius: radius(9.0), ..Default::default() },
        shadow: Shadow { color: fade(rgb(EMBER[0]), 0.45), offset: Vector::new(0.0, 3.0), blur_radius: 12.0 },
        ..Default::default()
    }
}

pub fn ring(size: f32, alpha: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(white(alpha * 0.35))),
        border: Border { color: white(alpha), width: 1.0, radius: radius(size / 2.0) },
        ..Default::default()
    }
}

pub fn orb(size: f32, hovering: bool) -> container::Style {
    container::Style {
        background: Some(linear(135.0, &EMBER)),
        border: Border { color: white(0.35), width: 1.0, radius: radius(size / 2.0) },
        shadow: Shadow {
            color: fade(rgb(EMBER[0]), if hovering { 0.8 } else { 0.5 }),
            offset: Vector::new(0.0, 8.0),
            blur_radius: if hovering { 60.0 } else { 40.0 },
        },
        ..Default::default()
    }
}

// ------------------------------------------------------------------ pills

#[derive(Clone, Copy)]
pub enum Tone {
    Green,
    Blue,
    Grey,
    Warn,
    Danger,
}

impl Tone {
    fn color(self) -> Color {
        match self {
            Tone::Green => GREEN,
            Tone::Blue => BLUE,
            Tone::Grey => white(0.75),
            Tone::Warn => AMBER,
            Tone::Danger => DANGER,
        }
    }
}

pub fn pill<'a>(label: impl text::IntoFragment<'a>, tone: Tone, a: f32) -> Element<'a, Message> {
    let c = tone.color();
    container(t(label).size(11).font(MEDIUM).color(fade(c, a)))
        .padding([3, 10])
        .style(move |_| container::Style {
            background: Some(Background::Color(fade(c, 0.16 * a))),
            border: Border { color: fade(c, 0.28 * a), width: 1.0, radius: radius(99.0) },
            ..Default::default()
        })
        .into()
}

// ------------------------------------------------------------------ buttons

/// The big round call to action at the bottom centre.
pub fn big_button<'a>(label: &'a str, on: Option<Message>, scale: f32) -> Element<'a, Message> {
    let size = 92.0 * scale;
    let button = button(t(label).size(17.0 * scale).font(BOLD).color(WHITE).center().width(Length::Fill).height(Length::Fill))
        .width(size)
        .height(size)
        .padding(0)
        .on_press_maybe(on)
        .style(move |_, s| {
            let (bg, glow, blur) = match s {
                button::Status::Disabled => (Background::Color(white(0.10)), Color::TRANSPARENT, 0.0),
                button::Status::Hovered => (linear(135.0, &[0xFF6A9E, 0xFFB15C]), fade(rgb(EMBER[0]), 0.75), 46.0),
                button::Status::Pressed => (linear(135.0, &[0xE8437A, 0xEE8A30]), fade(rgb(EMBER[0]), 0.5), 24.0),
                button::Status::Active => (linear(135.0, &EMBER), fade(rgb(EMBER[0]), 0.55), 34.0),
            };
            button::Style {
                background: Some(bg),
                text_color: WHITE,
                border: Border {
                    color: white(if s == button::Status::Disabled { 0.18 } else { 0.4 }),
                    width: 1.0,
                    radius: radius(size / 2.0),
                },
                shadow: Shadow { color: glow, offset: Vector::new(0.0, 8.0), blur_radius: blur },
                ..Default::default()
            }
        });
    // Fixed footprint so the pop doesn't shift the layout around it.
    iced::widget::container(button).width(110).height(110).center_x(110).center_y(110).into()
}

pub fn glass_button<'a>(label: &'a str, on: Message) -> Element<'a, Message> {
    button(t(label).size(12).font(MEDIUM).color(WHITE))
        .padding([6, 14])
        .on_press(on)
        .style(|_, s| button::Style {
            background: Some(Background::Color(white(if s == button::Status::Hovered { 0.16 } else { 0.10 }))),
            text_color: WHITE,
            border: Border { color: white(0.2), width: 1.0, radius: radius(99.0) },
            ..Default::default()
        })
        .into()
}

pub fn link_button<'a>(label: &'a str, on: Message) -> Element<'a, Message> {
    button(t(label).size(13).font(MEDIUM))
        .padding([6, 10])
        .on_press(on)
        .style(|_, s| button::Style {
            background: (s == button::Status::Hovered).then_some(Background::Color(white(0.08))),
            text_color: if s == button::Status::Hovered { WHITE } else { TEXT_2 },
            border: Border { radius: radius(8.0), ..Default::default() },
            ..Default::default()
        })
        .into()
}

pub fn ghost_button(s: button::Status) -> button::Style {
    button::Style {
        background: (s == button::Status::Hovered).then_some(Background::Color(white(0.1))),
        border: Border { radius: radius(6.0), ..Default::default() },
        ..Default::default()
    }
}

pub fn nav_button(active: bool, s: button::Status) -> button::Style {
    let bg = if active {
        Some(Background::Color(white(0.10)))
    } else if s == button::Status::Hovered {
        Some(Background::Color(white(0.05)))
    } else {
        None
    };
    button::Style { background: bg, border: Border { radius: radius(10.0), ..Default::default() }, ..Default::default() }
}

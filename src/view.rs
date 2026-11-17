//! What the app draws. Layout follows tools like livesaver: a quiet sidebar,
//! a top bar with the page's title and its main action on the right, and the
//! page below — steps to explain an empty page, tables once there's data.

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Column, Space, button, center, checkbox, column, container, opaque, row, scrollable, stack, svg, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::check::Format;
use crate::library::{self, Host};
use crate::{upgrades, vst2to3};
use crate::style::*;
use crate::*;

const BAR: f32 = 56.0;

/// The logo (`assets/logo.svg`; the app icons are rendered from it by `tools/icons.sh`).
const LOGO: &[u8] = include_bytes!("../assets/logo.svg");

/// Row reveal: each table row a beat after the one above.
const ROW_STAGGER: f32 = 0.025;

fn scroll<'a>(content: impl Into<Element<'a, Message>>) -> scrollable::Scrollable<'a, Message> {
    scrollable(content).direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4)))
}

/// A plugin across all projects: one table row.
struct Row {
    plugin: Plugin,
    status: Status,
    fix: Option<Fix>,
    instances: usize,
}

impl Row {
    /// FL opens it as it is: an installed VST3, or an installed VST2.
    fn opens(&self) -> bool {
        matches!(self.status, Status::Installed | Status::Vst2 { installed: Some(true) })
    }
    /// Shown under 需要处理: it won't open, there's an upgrade to an installed
    /// VST3, or a newer product to move to (升级插件).
    fn attention(&self) -> bool {
        !self.opens() || self.fix.as_ref().is_some_and(|f| f.default_on() || matches!(f, Fix::Migrate { .. }))
    }
}

/// Table order: what needs attention first.
fn rank(status: &Status) -> u8 {
    match status {
        Status::Missing => 0,
        Status::Mismatch => 1,
        Status::Rosetta => 2,
        Status::Vst2 { .. } => 3,
        Status::Installed => 4,
    }
}

fn format_label(f: Format) -> &'static str {
    match f {
        Format::Vst2 => "VST2",
        Format::Vst3 => "VST3",
    }
}

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let (title, actions, page) = match self.page {
            Page::Check => self.check_page(),
            Page::Library => self.library_page(),
            Page::Supported => ("官方支持", None, self.supported_page()),
        };
        let off = self.page_offset();
        let page = container(page).padding(Padding { top: off.max(0.0), bottom: (-off).max(0.0), ..Padding::ZERO });
        let mut bar = row![t(title).size(14).font(SEMIBOLD).color(p().text), Space::new().width(Length::Fill)]
            .spacing(8)
            .align_y(Alignment::Center);
        if let Some(a) = actions {
            bar = bar.push(a);
        }
        let body = row![
            self.sidebar(),
            column![
                container(bar).height(BAR).padding([0, 24]).center_y(BAR).width(Length::Fill).style(|_| top_bar()),
                hairline(),
                container(page).width(Length::Fill).height(Length::Fill).padding([20, 28]),
            ]
            .width(Length::Fill),
        ];
        let base = container(body).style(|_| background());
        match &self.picking {
            Some(plugin) => stack![base, opaque(container(center(self.picker(plugin))).style(|_| backdrop()))].into(),
            None => base.into(),
        }
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let p = p();
        let nav_item = |label: &'static str, glyph: &'static str, page: Page, count: usize| {
            let active = self.page == page;
            let mut line = row![
                container(t(glyph).size(13).color(if active { p.text } else { p.text_3 })).width(20).center_x(20),
                t(label).size(13).font(if active { SEMIBOLD } else { MEDIUM }).color(if active { p.text } else { p.text_2 }),
                Space::new().width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if count > 0 {
                line = line.push(
                    container(t(count.to_string()).size(11).font(MEDIUM).color(p.text_2))
                        .padding([0, 6])
                        .style(move |_| count_badge(active)),
                );
            }
            button(line).width(Length::Fill).padding([7, 8]).on_press(Message::Nav(page)).style(move |_, s| nav(active, s))
        };
        let section = |label: &'static str| container(t(label).size(11).font(MEDIUM).color(p.text_3)).padding([8, 8]);
        let logo_row = row![
            svg(svg::Handle::from_memory(LOGO)).width(30).height(30),
            column![t("Blacksmith").size(14).font(BOLD).color(p.text), t("工程文件医生").size(11).color(p.text_3)],
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl " };
        let shortcut = move |keys: String, what: &'static str| {
            row![t(what).size(11).color(p.text_3), Space::new().width(Length::Fill), t(keys).size(11).font(MEDIUM).color(p.text_2)]
        };
        let found = |h: Host| matches!(library::get(h).source(), library::Source::Host(_));
        let source = match (found(Host::Fl), found(Host::Live)) {
            (true, true) => "插件库：FL + Live 数据库",
            (true, false) => "插件库：FL 数据库",
            (false, true) => "插件库：Live 数据库",
            (false, false) => "插件库：插件文件夹",
        };

        container(
            column![
                container(logo_row).padding([0, 6]),
                Space::new().height(18),
                nav_item("检查工程", "◎", Page::Check, 0),
                Space::new().height(6),
                section("插件"),
                nav_item("已安装插件", "▤", Page::Library, 0),
                nav_item("官方支持", "✓", Page::Supported, plugins::RULES.len() + vst2to3::rules().len() + upgrades::UPGRADES.len()),
                Space::new().height(Length::Fill),
                container(
                    column![
                        shortcut(format!("{cmd}O"), "打开工程"),
                        shortcut(format!("{cmd}↵"), "应用"),
                        shortcut(format!("{cmd}F"), "搜索插件"),
                    ]
                    .spacing(4),
                )
                .padding([0, 8]),
                Space::new().height(10),
                hairline(),
                container(status(source, Tone::Grey, 1.0)).padding(Padding { top: 10.0, left: 8.0, ..Padding::ZERO }),
            ]
            .spacing(2),
        )
        .width(224)
        .height(Length::Fill)
        .padding(Padding { top: 16.0, right: 10.0, bottom: 14.0, left: 10.0 })
        .style(|_| sidebar())
        .into()
    }

    /// Big title and a line under it, at the top of a page's content.
    fn intro<'a>(&self, title: &'a str, about: impl Into<String>) -> Element<'a, Message> {
        let p = p();
        column![t(title).size(24).font(BOLD).color(p.text), t(about.into()).size(13).color(p.text_2)].spacing(6).into()
    }

    /// The host of the open project (FL Studio when none) and its library.
    fn project_host(&self) -> Host {
        self.project.as_ref().map_or(Host::Fl, |i| i.state.host())
    }

    /// (opacity, springy progress) of the n-th table row.
    fn row_reveal(&self, n: usize) -> (f32, f32) {
        let tt = self.since(self.list_at) - ROW_STAGGER * n.min(24) as f32;
        (ease_out_cubic(tt / FADE), ease_out_back(tt / SLIDE))
    }

    // ---------------------------------------------------------- 检查工程

    fn check_page(&self) -> (&'static str, Option<Element<'_, Message>>, Element<'_, Message>) {
        let p = p();
        let title = "检查工程";
        let Some(item) = &self.project else {
            let open = button(t("打开工程…").size(13).font(MEDIUM)).padding([7, 12]).on_press(Message::Browse).style(|_, s| secondary(s));
            return (title, Some(open.into()), self.empty_check_page());
        };

        let rows = self.plugin_rows();
        let opens = rows.iter().filter(|r| r.opens()).count();
        let rosetta = rows.iter().filter(|r| r.status == Status::Rosetta).count();
        let missing = rows.len() - opens - rosetta;
        let attention = rows.iter().filter(|r| r.attention()).count();
        let migrate = |r: &Row| matches!(r.fix, Some(Fix::Migrate { .. }));
        let upgrades = rows.iter().filter(|r| r.opens() && r.attention() && !migrate(r)).count();
        let newer = rows.iter().filter(|r| migrate(r)).count();
        let pending = self.pending();

        // The project: name, folder, and where things stand.
        let file = item.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = item.path.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let mut head = row![
            column![t(file).size(18).font(BOLD).color(p.text), t(dir).size(11).color(p.text_3)].spacing(3).width(Length::Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        match &item.state {
            ProjectState::Saved { out, .. } => {
                head = head.push(status(item.state.headline(), Tone::Green, 1.0));
                head = head.push(
                    button(t("在文件夹中显示").size(12).font(MEDIUM))
                        .padding([5, 10])
                        .on_press(Message::Reveal(out.clone()))
                        .style(|_, s| secondary(s)),
                );
            }
            ProjectState::Failed(e) => head = head.push(status(e.clone(), Tone::Red, 1.0)),
            ProjectState::Checked { .. } => {}
        }

        let mut summary = row![t(format!("{} 个插件", rows.len())).size(13).color(p.text_2)].spacing(16).align_y(Alignment::Center);
        summary = summary.push(status(format!("{opens} 个能直接打开"), Tone::Green, 1.0));
        if missing > 0 {
            summary = summary.push(status(format!("{missing} 个打不开"), Tone::Red, 1.0));
        }
        if rosetta > 0 {
            summary = summary.push(status(format!("{rosetta} 个只有 Intel 版"), Tone::Amber, 1.0));
        }
        if upgrades > 0 {
            summary = summary.push(status(format!("{upgrades} 个可以升级到 VST3"), Tone::Blue, 1.0));
        }
        if newer > 0 {
            summary = summary.push(status(format!("{newer} 个可以换成新版插件"), Tone::Blue, 1.0));
        }
        if self.scanning > 0 {
            summary = summary.push(status("正在读取…", Tone::Grey, 1.0));
        }
        let seg = |label: String, active: bool, msg: Message| {
            button(t(label).size(12).font(if active { SEMIBOLD } else { MEDIUM }))
                .padding([5, 12])
                .on_press(msg)
                .style(move |_, s| style::tab(active, s))
        };
        let filter = container(
            row![
                seg(format!("需要处理 {attention}"), self.filter == Filter::Attention, Message::Filter(Filter::Attention)),
                seg(format!("全部 {}", rows.len()), self.filter == Filter::All, Message::Filter(Filter::All)),
            ]
            .spacing(2),
        )
        .padding(2)
        .style(|_| style::tabs());
        let controls = row![summary, Space::new().width(Length::Fill), filter].align_y(Alignment::Center);

        let apply = button(t(if pending > 0 { format!("应用 {pending} 处修改") } else { "应用".to_string() }).size(13).font(SEMIBOLD))
            .padding([7, 14])
            .on_press_maybe((pending > 0).then_some(Message::Apply))
            .style(|_, s| primary(s));
        let other = button(t("换一个工程…").size(13).font(MEDIUM)).padding([7, 12]).on_press(Message::Browse).style(|_, s| secondary(s));
        let close = button(t("关闭").size(13).font(MEDIUM)).padding([7, 10]).on_press(Message::Clear).style(|_, s| ghost(s));
        let actions = row![close, other, apply].spacing(8).align_y(Alignment::Center);

        let host = self.project_host();
        let short = if host == Host::Fl { "FL" } else { "Live" };
        let ext = item.path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let hint = t(if self.hovering {
            "松开就换成这个工程".to_string()
        } else {
            format!(
                "勾选的修改会另存为「名字 (修复).{ext}」，放在原文件旁边，原文件不动。对照的是这台电脑上 {} 的插件库。「未验证」：{short} 能找到插件，但存档能否读进去没在 {short} 里试过。",
                host.name()
            )
        })
        .size(12)
        .color(if self.hovering { p.text } else { p.text_3 });
        let page =
            column![head, controls, container(self.plugin_table(rows)).height(Length::Fill).style(|_| card(1.0)), hint].spacing(14).into();
        (title, Some(actions.into()), page)
    }

    fn empty_check_page(&self) -> Element<'_, Message> {
        let p = p();
        let steps = [
            ("打开", "拖入一个 FL Studio 工程（.flp）或 Ableton Live 工程（.als），或点「选择工程」。"),
            ("检查", "对照这台电脑上 FL 或 Live 自己的插件数据库：哪些能直接打开，哪些是 VST2、ID 对不上或没装。这一步只读。"),
            ("应用", "勾选要做的修改：升级到 VST3、改成已装的插件。另存为「名字 (修复).flp」。"),
        ];
        let mut cards = row![].spacing(12);
        for (i, (name, text)) in steps.into_iter().enumerate() {
            cards = cards.push(
                container(
                    column![
                        row![
                            container(t((i + 1).to_string()).size(12).font(SEMIBOLD).color(p.text_2))
                                .width(24)
                                .height(24)
                                .center_x(24)
                                .center_y(24)
                                .style(|_| step_number()),
                            t(name).size(14).font(SEMIBOLD).color(p.text),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                        t(text).size(12).color(p.text_2),
                    ]
                    .spacing(10),
                )
                .padding(14)
                .width(Length::FillPortion(1))
                .style(|_| card(1.0)),
            );
        }
        let h = self.hovering;
        let d = 64.0 * self.icon_scale();
        let icon = container(t("↓").size(28.0 * d / 64.0).font(BOLD).color(if h { p.panel } else { p.text_2 }))
            .width(d)
            .height(d)
            .center_x(d)
            .center_y(d)
            .style(move |_| icon_circle(d, h));
        let zone = container(center(
            column![
                container(icon).width(96).height(96).center_x(96).center_y(96),
                t(if self.scanning > 0 {
                    "正在读取工程…"
                } else if h {
                    "松开就开始检查"
                } else {
                    "把一个 .flp 或 .als 工程拖到这里"
                })
                .size(16)
                .font(SEMIBOLD)
                .color(p.text),
                t("只读不写；你确认之后才会另存新文件").size(12).color(p.text_3),
                Space::new().height(10),
                button(t("选择工程…").size(13).font(SEMIBOLD)).padding([8, 16]).on_press(Message::Browse).style(|_, s| primary(s)),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| drop_zone(h));
        column![
            self.intro("检查工程", "看看工程里的每个插件在这台电脑上能不能打开，再决定怎么修。"),
            cards,
            zone
        ]
        .spacing(18)
        .into()
    }

    /// Every plugin in the project, needing attention first.
    fn plugin_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .project
            .iter()
            .flat_map(|i| i.state.findings())
            .map(|f| Row { plugin: f.plugin.clone(), status: f.status.clone(), fix: f.fix.clone(), instances: f.instances })
            .collect();
        rows.sort_by(|a, b| {
            (!a.attention(), rank(&a.status))
                .cmp(&(!b.attention(), rank(&b.status)))
                .then_with(|| a.plugin.name.to_lowercase().cmp(&b.plugin.name.to_lowercase()))
        });
        rows
    }

    fn plugin_table(&self, rows: Vec<Row>) -> Element<'_, Message> {
        let p = p();
        let header = row![
            Space::new().width(18),
            th("插件").width(Length::FillPortion(4)),
            th("格式").width(44),
            th("状态").width(Length::FillPortion(3)),
            th("可以做什么").width(Length::FillPortion(5)),
            th("实例").width(40).align_x(Alignment::End),
        ]
        .spacing(12)
        .padding([10, 14]);
        let rows: Vec<Row> = rows.into_iter().filter(|r| self.filter == Filter::All || r.attention()).collect();
        let mut body = Column::new();
        if rows.is_empty() {
            body = body.push(
                container(t("这些工程里的插件都能直接打开").size(13).color(p.text_3)).center_x(Length::Fill).padding(28),
            );
        }
        for (i, r) in rows.into_iter().enumerate() {
            let (a, k) = self.row_reveal(i);
            let finding = Finding { plugin: r.plugin.clone(), status: r.status.clone(), fix: r.fix.clone(), instances: r.instances };
            let choice = self.choices.get(&r.plugin).copied();
            let can_tick = r.fix.is_some() || matches!(choice, Some(Choice::Pick { .. }));
            let tick: Element<'_, Message> = if can_tick {
                let plugin = r.plugin.clone();
                checkbox(self.ticked(&finding)).size(14).on_toggle(move |on| Message::Tick(plugin.clone(), on)).style(|_, s| tick(s)).into()
            } else {
                Space::new().width(14).into()
            };
            let (label, tone) = status_label(&r.status);
            let mut name = row![t(r.plugin.name.clone()).size(13).font(MEDIUM).color(fade(p.text, a))].spacing(6).align_y(Alignment::Center);
            if !r.plugin.vendor.is_empty() {
                name = name.push(t(r.plugin.vendor.clone()).size(11).color(fade(p.text_3, a)));
            }

            // What can be done: the suggested fix or the user's pick, its
            // standing, and a way to pick another installed plugin.
            let pick_label = |s: &'static str| {
                let plugin = r.plugin.clone();
                button(t(s).size(11).font(MEDIUM)).padding([2, 8]).on_press(Message::Pick(Some(plugin))).style(|_, s| secondary(s))
            };
            // A Live set's VST2 can't be moved to a VST3 yet (needs its state converted).
            let can_pick = !(self.project_host() == Host::Live && r.plugin.format == Format::Vst2);
            let mut todo = row![].spacing(8).align_y(Alignment::Center);
            match (choice, &r.fix) {
                (Some(Choice::Pick { id, .. }), _) => {
                    let target = library::get(self.project_host()).by_class_id(&id).map(|k| k.name).unwrap_or("?");
                    todo = todo.push(t(format!("改成已装的 {target}")).size(12).color(fade(p.text, a)));
                    todo = todo.push(t("你选的 · 未验证").size(11).color(fade(p.text_3, a)));
                    todo = todo.push(pick_label("更换…"));
                }
                (_, Some(fix)) => {
                    todo = todo.push(t(fix_label(fix)).size(12).color(fade(p.text, a)));
                    todo = todo.push(if fix.verified() {
                        t("官方支持").size(11).color(fade(p.green, a))
                    } else {
                        t("未验证").size(11).color(fade(p.text_3, a))
                    });
                    if !r.opens() && !fix.verified() && can_pick {
                        todo = todo.push(pick_label("更换…"));
                    }
                }
                (_, None) if !r.opens() && can_pick => {
                    todo = todo.push(pick_label("选择已装的插件…"));
                }
                (_, None) if !r.opens() => {
                    todo = todo.push(t("Live 工程里的 VST2 还不能换成别的插件").size(11).color(fade(p.text_3, a)));
                }
                _ => todo = todo.push(t("—").size(12).color(fade(p.text_3, a))),
            }

            // An unverified change that's ticked: keep the settings (the plugin
            // may not read them, and can crash the host) or reset them.
            let unverified = matches!(choice, Some(Choice::Pick { .. })) || r.fix.as_ref().is_some_and(|f| !f.verified());
            let mut todo = column![todo].spacing(6);
            if unverified && self.ticked(&finding) {
                let reset = choice.is_some_and(Choice::reset);
                let seg = |label: &'static str, active: bool, to: bool| {
                    let plugin = r.plugin.clone();
                    button(t(label).size(11).font(if active { SEMIBOLD } else { MEDIUM }))
                        .padding([2, 8])
                        .on_press(Message::Reset(plugin, to))
                        .style(move |_, s| style::tab(active, s))
                };
                let toggle = container(row![seg("保留设置", !reset, false), seg("重置设置", reset, true)].spacing(2))
                    .padding(2)
                    .style(|_| style::tabs());
                let note = if reset {
                    t("插件以默认设置打开，原来的设置会丢").size(11).color(fade(p.text_3, a))
                } else {
                    t("插件可能读不了原来的设置，严重时会让宿主崩溃").size(11).color(fade(p.amber, a))
                };
                todo = todo.push(row![toggle, note].spacing(8).align_y(Alignment::Center));
            }

            if i > 0 {
                body = body.push(hairline());
            }
            body = body.push(
                row![
                    container(tick).width(18),
                    name.width(Length::FillPortion(4)),
                    t(format_label(r.plugin.format)).size(12).color(fade(p.text_2, a)).width(44),
                    status(label, tone, a).width(Length::FillPortion(3)),
                    todo.width(Length::FillPortion(5)),
                    t(r.instances.to_string()).size(12).color(fade(p.text_2, a)).width(40).align_x(Alignment::End),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .padding(Padding { left: 14.0 + 10.0 * (1.0 - k), right: 14.0, top: 8.0, bottom: 8.0 }),
            );
        }
        column![header, hairline(), scroll(body).height(Length::Fill)].into()
    }

    /// The dialog for pointing a plugin at an installed VST3.
    fn picker<'a>(&'a self, plugin: &'a Plugin) -> Element<'a, Message> {
        let p = p();
        let q = self.query_key(&self.pick_query);
        let lib = library::get(self.project_host());
        let own = library::plugin_key(&plugin.name, &plugin.vendor);
        let vendor = library::plugin_key(&plugin.vendor, "");
        // Likely matches first: same vendor, then a similar name.
        let mut list: Vec<(u8, &library::Entry)> = lib
            .all()
            .iter()
            .filter(|k| q.is_empty() || k.name.to_lowercase().contains(&q) || k.vendor.to_lowercase().contains(&q))
            .map(|k| {
                let key = library::plugin_key(k.name, k.vendor);
                let same_vendor = !vendor.is_empty() && library::plugin_key(k.vendor, "") == vendor;
                let similar = !own.is_empty() && (key.contains(&own) || own.contains(&key));
                (u8::from(!same_vendor) + u8::from(!similar), k)
            })
            .collect();
        list.sort_by_key(|(score, k)| (*score, k.vendor.to_lowercase(), k.name.to_lowercase()));

        let mut rows = Column::new();
        for (i, (_, k)) in list.iter().take(300).enumerate() {
            if i > 0 {
                rows = rows.push(hairline());
            }
            rows = rows.push(
                button(
                    row![
                        t(k.name).size(13).font(MEDIUM).color(p.text),
                        t(k.vendor).size(11).color(p.text_3),
                        Space::new().width(Length::Fill),
                        t(if k.instrument { "乐器" } else { "效果器" }).size(11).color(p.text_3),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([7, 12])
                .on_press(Message::PickTarget(k.class_id))
                .style(|_, s| ghost(s)),
            );
        }
        if list.is_empty() {
            rows = rows.push(container(t("没有找到匹配的插件").size(13).color(p.text_3)).center_x(Length::Fill).padding(24));
        }
        let what = match plugin.format {
            Format::Vst2 => "转换成你选的 VST3，存档按 VST3 规范带过去",
            Format::Vst3 => "改成你选的插件的 ID，存档原样带过去",
        };
        container(
            column![
                t(format!("给「{}」选一个已装的插件", plugin.name)).size(16).font(SEMIBOLD).color(p.text),
                t(format!("{what}。能否读进去没验证过，打开后检查一下设置。")).size(12).color(p.text_2),
                text_input("搜索插件或厂商…", &self.pick_query)
                    .id(PICK_SEARCH)
                    .on_input(Message::PickSearch)
                    .padding([8, 12])
                    .size(13)
                    .style(|_, s| search_field(s)),
                container(scroll(rows).height(Length::Fill)).height(Length::Fill).style(|_| card(1.0)),
                row![
                    Space::new().width(Length::Fill),
                    button(t("取消").size(13).font(MEDIUM)).padding([7, 14]).on_press(Message::Pick(None)).style(|_, s| secondary(s)),
                ],
            ]
            .spacing(12),
        )
        .padding(20)
        .width(560)
        .height(500)
        .style(|_| dialog())
        .into()
    }

    fn query_key(&self, q: &str) -> String {
        q.trim().to_lowercase()
    }

    // ---------------------------------------------------------- 官方支持

    fn supported_page(&self) -> Element<'_, Message> {
        let p = p();
        let rules_header = row![
            th("插件").width(Length::FillPortion(3)),
            th("厂商").width(Length::FillPortion(3)),
            th("说明").width(Length::FillPortion(6)),
        ]
        .spacing(12)
        .padding([10, 14]);
        let mut rules = column![rules_header, hairline()];
        for (i, r) in plugins::RULES.iter().enumerate() {
            if i > 0 {
                rules = rules.push(hairline());
            }
            let mut note = r.note.to_string();
            if !r.aliases.is_empty() {
                note.push_str(" · 也能修复旧版 VST3 的 ID");
            }
            rules = rules.push(
                row![
                    status(r.name, Tone::Green, 1.0).width(Length::FillPortion(3)),
                    t(r.vendor).size(13).color(p.text_2).width(Length::FillPortion(3)),
                    t(note).size(12).color(p.text_2).width(Length::FillPortion(6)),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .padding([10, 14]),
            );
        }

        // Live sets: VST2 → VST3 of the same plugin (assets/vst2to3.tsv), by vendor.
        let mut live_rules: Vec<&vst2to3::Rule> = vst2to3::rules().iter().collect();
        live_rules.sort_by_key(|r| (r.vendor.to_lowercase(), r.name.to_lowercase()));
        let mut live = column![
            row![
                th("插件").width(Length::FillPortion(3)),
                th("厂商").width(Length::FillPortion(3)),
                th("说明").width(Length::FillPortion(6)),
            ]
            .spacing(12)
            .padding([10, 14]),
            hairline()
        ];
        for (i, r) in live_rules.into_iter().enumerate() {
            if i > 0 {
                live = live.push(hairline());
            }
            let about = match (r.verified, r.note) {
                (true, _) => "设置原样迁移，已在 Ableton Live 12 验证".to_string(),
                (false, "") => "未验证 · 默认不勾".to_string(),
                (false, note) => format!("未验证 · {note} · 默认不勾"),
            };
            live = live.push(
                row![
                    status(r.name, if r.verified { Tone::Green } else { Tone::Blue }, 1.0).width(Length::FillPortion(3)),
                    t(r.vendor).size(13).color(p.text_2).width(Length::FillPortion(3)),
                    t(about).size(12).color(p.text_2).width(Length::FillPortion(6)),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .padding([8, 14]),
            );
        }

        // 升级插件: moving to a different, newer product.
        let up_header = row![
            th("从").width(Length::FillPortion(3)),
            th("升级到").width(Length::FillPortion(3)),
            th("说明").width(Length::FillPortion(6)),
        ]
        .spacing(12)
        .padding([10, 14]);
        let mut ups = column![up_header, hairline()];
        for (i, u) in upgrades::UPGRADES.iter().enumerate() {
            if i > 0 {
                ups = ups.push(hairline());
            }
            ups = ups.push(
                row![
                    t(u.from_name).size(13).color(p.text).width(Length::FillPortion(3)),
                    status(u.to_name, if u.verified { Tone::Green } else { Tone::Blue }, 1.0).width(Length::FillPortion(3)),
                    t(format!("{} · {}{}", u.vendor, u.note, if u.verified { "" } else { " · 未验证" }))
                        .size(12)
                        .color(p.text_2)
                        .width(Length::FillPortion(6)),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .padding([10, 14]),
            );
        }

        let legend: [(Status, &str); 7] = [
            (Status::Installed, "FL 按格式和 ID 能在这台电脑上找到它"),
            (Status::Mismatch, "工程里存的 VST3 ID 这台电脑上没有，但装着同名同厂商的插件；可以改成它"),
            (Status::Missing, "ID 和名字都对不上，多半是没装；可以手动选一个已装的插件"),
            (Status::Vst2 { installed: Some(true) }, "VST2，这台电脑上装着；可以升级到 VST3"),
            (Status::Vst2 { installed: Some(false) }, "VST2，这台电脑上没装；装着 VST3 的话可以升级"),
            (Status::Vst2 { installed: None }, "VST2；没找到宿主的插件数据库，不知道装没装"),
            (Status::Rosetta, "装着，但只有 Intel 版：Apple 芯片的 Mac 上要用 Rosetta 打开宿主才能加载；有原生的 VST3 的话可以升级过去"),
        ];
        let mut explain = column![row![th("状态").width(200), th("含义").width(Length::Fill)].spacing(12).padding([10, 14]), hairline()];
        for (i, (s, text)) in legend.into_iter().enumerate() {
            let (label, tone) = status_label(&s);
            if i > 0 {
                explain = explain.push(hairline());
            }
            explain = explain.push(
                row![status(label, tone, 1.0).width(200), t(text).size(12).color(p.text_2).width(Length::Fill)]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .padding([9, 14]),
            );
        }

        scroll(
            column![
                self.intro("官方支持", "这些插件有专门的规则，在宿主里验证过：音色和设置原样迁移。其他插件的修改标为「未验证」。"),
                t("FL Studio · VST2 → VST3").size(15).font(SEMIBOLD).color(p.text),
                container(rules).style(|_| card(1.0)),
                Space::new().height(4),
                t("Ableton Live · VST2 → VST3").size(15).font(SEMIBOLD).color(p.text),
                t("Live 工程里的 VST2 换成同一个插件的 VST3。转换后的工程经 Live 重新保存，和插件自己的 VST3 存的设置一致。")
                    .size(12)
                    .color(p.text_2),
                container(live).style(|_| card(1.0)),
                Space::new().height(4),
                t("升级插件").size(15).font(SEMIBOLD).color(p.text),
                t("把工程里的插件换成另一个更新的产品（比如 Kontakt → Kontakt 8）。目标插件装着时，检查工程会把它列为可选修改，默认不勾。")
                    .size(12)
                    .color(p.text_2),
                container(ups).style(|_| card(1.0)),
                Space::new().height(4),
                t("状态说明").size(15).font(SEMIBOLD).color(p.text),
                container(explain).style(|_| card(1.0)),
                container(
                    t("想加新的官方支持：提供一个工程，里面同一个插件的 VST2 和 VST3 都加载了同一个预设。").size(12).color(p.text_2),
                )
                .padding(12)
                .width(Length::Fill)
                .style(|_| note()),
            ]
            .spacing(14)
            .padding(Padding { right: 12.0, ..Padding::ZERO }),
        )
        .height(Length::Fill)
        .into()
    }

    // ---------------------------------------------------------- 已安装插件

    /// Every installed plugin, grouped by vendor like a DAW's browser.
    fn library_page(&self) -> (&'static str, Option<Element<'_, Message>>, Element<'_, Message>) {
        let p = p();
        let query = self.query_key(&self.query);
        let searching = !query.is_empty();

        // (vendor as shown, lower-case key, plugins), in library order.
        let lib = library::get(self.library_host);
        let mut vendors: Vec<(&str, String, Vec<&library::Entry>)> = Vec::new();
        for k in lib.all() {
            let key = k.vendor.to_lowercase();
            if searching && !k.name.to_lowercase().contains(&query) && !key.contains(&query) {
                continue;
            }
            match vendors.iter_mut().find(|(_, v, _)| *v == key) {
                Some((_, _, list)) => list.push(k),
                None => vendors.push((k.vendor, key, vec![k])),
            }
        }
        let total = lib.all().len();
        let vendor_count = {
            let mut v: Vec<String> = lib.all().iter().map(|k| k.vendor.to_lowercase()).collect();
            v.sort();
            v.dedup();
            v.len()
        };
        let shown: usize = vendors.iter().map(|(_, _, l)| l.len()).sum();

        let all_open = !searching && vendors.iter().all(|(_, key, _)| self.open.contains(key));
        let expand = button(t(if all_open { "全部收起" } else { "全部展开" }).size(13).font(MEDIUM))
            .padding([7, 12])
            .on_press(Message::ExpandAll(!all_open))
            .style(|_, s| secondary(s));

        let search = text_input("搜索插件或厂商…", &self.query)
            .id(SEARCH)
            .on_input(Message::Search)
            .padding([8, 12])
            .size(13)
            .style(|_, s| search_field(s));
        let counter = t(if searching { format!("{shown} / {total}") } else { format!("{total} 个插件 · {vendor_count} 个厂商") })
            .size(12)
            .color(p.text_3);

        let header = row![
            th("插件").width(Length::FillPortion(5)),
            th("类型").width(70),
            th("格式").width(Length::FillPortion(2)),
            th("状态").width(Length::FillPortion(2)),
        ]
        .spacing(12)
        .padding(Padding { left: 38.0, right: 14.0, top: 10.0, bottom: 10.0 });

        let mut list = Column::new();
        for (n, (vendor, key, plugins)) in vendors.iter().enumerate() {
            let open = searching || self.open.contains(key);
            let official = plugins.iter().filter(|k| k.fourcc.and_then(plugins::find).is_some()).count();
            let mut head = row![
                container(t(if open { "▾" } else { "▸" }).size(11).color(p.text_3)).width(14),
                t(*vendor).size(13).font(SEMIBOLD).color(p.text),
                t(plugins.len().to_string()).size(12).color(p.text_3),
                Space::new().width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if official > 0 {
                head = head.push(status(format!("{official} 个官方支持"), Tone::Green, 1.0));
            }
            if n > 0 {
                list = list.push(hairline());
            }
            list = list.push(
                button(head)
                    .width(Length::Fill)
                    .padding([8, 12])
                    .on_press_maybe((!searching).then(|| Message::ToggleVendor(key.clone())))
                    .style(|_, s| ghost(s)),
            );
            if !open {
                continue;
            }
            // Children pop in one after another when a vendor is opened.
            let since = match &self.opened {
                Some((v, at)) if v == key && !searching => self.since(*at),
                _ => 10.0,
            };
            for (i, k) in plugins.iter().enumerate() {
                let tt = since - ROW_STAGGER * i as f32;
                let (a, slide) = (ease_out_cubic(tt / FADE), ease_out_back(tt / SLIDE));
                let format = if k.fourcc.is_some() { "VST2 → VST3" } else { "VST3" };
                let state: Element<'_, Message> = if lib.intel_only(k) {
                    status("仅 Intel", Tone::Amber, a).into()
                } else if k.fourcc.and_then(plugins::find).is_some() {
                    status("官方支持", Tone::Green, a).into()
                } else {
                    Space::new().into()
                };
                list = list.push(
                    row![
                        t(k.name).size(13).color(fade(p.text, a)).width(Length::FillPortion(5)),
                        t(if k.instrument { "乐器" } else { "效果器" }).size(12).color(fade(p.text_2, a)).width(70),
                        t(format).size(12).color(fade(p.text_2, a)).width(Length::FillPortion(2)),
                        container(state).width(Length::FillPortion(2)),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .padding(Padding { left: 38.0 + 12.0 * (1.0 - slide), right: 14.0, top: 6.0, bottom: 6.0 }),
                );
            }
        }
        if vendors.is_empty() {
            list = list.push(container(t("没有找到匹配的插件").size(13).color(p.text_3)).center_x(Length::Fill).padding(28));
        }

        let host = self.library_host;
        let short = if host == Host::Fl { "FL" } else { "Live" };
        let scan = lib.scan();
        let folders = scan.folders.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join("、");
        let about = match lib.source() {
            library::Source::Host(dir) => format!(
                "以 {} 自己的插件数据库为准（{}），再对照插件文件夹（{folders}）核对。只读文件，不加载插件。",
                host.name(),
                dir.display()
            ),
            library::Source::Folders => {
                format!("没找到 {} 的插件数据库，用的是插件文件夹里能读到的信息（{folders}）。只读文件，不加载插件。", host.name())
            }
        };
        let mut facts = row![status(format!("插件文件夹里 {} 个插件", scan.on_disk), Tone::Grey, 1.0)].spacing(16);
        if scan.ids_from_files > 0 {
            facts = facts.push(status(format!("{} 个 ID 从插件自带的 moduleinfo.json 补上", scan.ids_from_files), Tone::Grey, 1.0));
        }
        if scan.stale > 0 {
            facts = facts.push(status(format!("{short} 数据库里 {} 个已删除的插件已忽略", scan.stale), Tone::Grey, 1.0));
        }
        if lib.intel_count() > 0 {
            facts = facts.push(status(format!("{} 个插件文件只有 Intel 版，需 Rosetta", lib.intel_count()), Tone::Amber, 1.0));
        }
        let seg = |h: Host| {
            let active = self.library_host == h;
            button(t(h.name()).size(12).font(if active { SEMIBOLD } else { MEDIUM }))
                .padding([5, 12])
                .on_press(Message::LibraryHost(h))
                .style(move |_, s| style::tab(active, s))
        };
        let hosts = container(row![seg(Host::Fl), seg(Host::Live)].spacing(2)).padding(2).style(|_| style::tabs());
        let mut page = column![self.intro("已安装插件", about), row![hosts, Space::new().width(Length::Fill), facts].align_y(Alignment::Center)]
            .spacing(14);
        if !scan.unregistered.is_empty() {
            let names = scan.unregistered.join("、");
            page = page.push(
                container(
                    column![
                        status(format!("{short} 还没扫描到 {} 个插件", scan.unregistered.len()), Tone::Amber, 1.0),
                        t(match host {
                            Host::Fl => format!("{names}。在 FL 的「选项 → 管理插件」里扫描一次，FL 才能在工程里打开它们。"),
                            Host::Live => format!("{names}。在 Live 的「设置 → 插件」里重新扫描一次，Live 才能在工程里打开它们。"),
                        })
                        .size(12)
                        .color(p.text_2),
                    ]
                    .spacing(6),
                )
                .padding(12)
                .width(Length::Fill)
                .style(|_| note()),
            );
        }
        let page = page
            .push(row![search, counter].spacing(12).align_y(Alignment::Center))
            .push(container(column![header, hairline(), scroll(list).height(Length::Fill)]).height(Length::Fill).style(|_| card(1.0)))
            .push(t("「VST2 → VST3」：这台电脑上 VST2 和 VST3 都装了，或已知它对应的 VST3。").size(12).color(p.text_3))
            .into();
        ("已安装插件", Some(expand.into()), page)
    }
}

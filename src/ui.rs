use crate::theme::Theme;
use global_network_map::{
    config::Origin,
    geo::{self, City, CONTINENTS},
    model::{bytes, Model},
};
use ratatui::{
    prelude::*,
    symbols::Marker,
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row,
        Sparkline, Table, Wrap,
    },
};
use std::{collections::HashMap, time::Instant};

pub const MIN_WIDTH: u16 = 120;
pub const MIN_HEIGHT: u16 = 35;
const SOUTH: f64 = -60.;
const NORTH: f64 = 85.;
type Coastline = Vec<Vec<[f64; 2]>>;
pub fn coastline() -> Coastline {
    serde_json::from_str(include_str!("../assets/coastline.json")).expect("bundled coastline")
}

pub struct View<'a> {
    pub model: &'a Model,
    pub origin: &'a Origin,
    pub theme: &'a Theme,
    pub coast: &'a Coastline,
    pub interface: &'a str,
    pub database: &'a str,
    pub status: &'a str,
    pub demo: bool,
    pub stopped: bool,
    pub animation: bool,
    pub help: bool,
    pub mouse: Option<(u16, u16)>,
    pub elapsed: f64,
}
#[derive(Clone)]
struct Node {
    city: String,
    country: String,
    x: f64,
    y: f64,
    rate: [f64; 2],
    total: [u64; 2],
    last: [Option<Instant>; 2],
    count: usize,
}
fn nodes(model: &Model) -> Vec<Node> {
    let mut groups: HashMap<String, Node> = HashMap::new();
    for (_, e) in model.active() {
        let Some((x, y)) = e.location.coordinates() else {
            continue;
        };
        let key = format!("{}:{}:{x:.2}:{y:.2}", e.location.city, e.location.country);
        let n = groups.entry(key).or_insert_with(|| Node {
            city: e.location.city.clone(),
            country: e.location.country.clone(),
            x,
            y,
            rate: [0.; 2],
            total: [0; 2],
            last: [None, None],
            count: 0,
        });
        n.count += 1;
        for i in 0..2 {
            n.rate[i] += e.counter.rate[i];
            n.total[i] += e.counter.total[i];
            n.last[i] = n.last[i].max(e.last[i]);
        }
    }
    let mut nodes: Vec<_> = groups.into_values().collect();
    nodes.sort_by(|a, b| {
        b.rate
            .iter()
            .sum::<f64>()
            .total_cmp(&a.rate.iter().sum::<f64>())
            .then_with(|| a.city.cmp(&b.city))
    });
    nodes
}
fn block<'a>(title: impl Into<Line<'a>>, t: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border))
        .title(title)
        .title_style(Style::default().fg(t.accent))
}
fn center(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}
pub fn too_small(frame: &mut Frame, t: &Theme) -> bool {
    let area = frame.area();
    if area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT {
        return false;
    }
    frame.render_widget(Paragraph::new(format!("GLOBAL NETWORK MAP\n\nResize your terminal to at least {MIN_WIDTH} × {MIN_HEIGHT}.\nCurrent size: {} × {}\n\nCapture continues while resizing.  q to quit",area.width,area.height))
        .alignment(Alignment::Center).wrap(Wrap{trim:false}).style(Style::default().fg(t.foreground).bg(t.background)),center(area,68,9));
    true
}
fn faded(color: Color, background: Color, ratio: f64) -> Color {
    if let (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) = (color, background) {
        let blend = |a: u8, b: u8| (f64::from(a) * ratio + f64::from(b) * (1. - ratio)) as u8;
        Color::Rgb(blend(r, br), blend(g, bg), blend(b, bb))
    } else {
        color
    }
}

pub fn arc(origin: (f64, f64), end: (f64, f64), t: f64, direction: usize) -> (f64, f64) {
    let delta = (end.0 - origin.0 + 540.).rem_euclid(360.) - 180.;
    let bend = (delta.abs() * 0.16 + 7.).min(26.);
    let y = origin.1
        + (end.1 - origin.1) * t
        + 4. * t * (1. - t) * (bend + if direction == 0 { 2. } else { -2. });
    (
        (origin.0 + delta * t + 180.).rem_euclid(360.) - 180.,
        y.clamp(SOUTH + 1., NORTH - 1.),
    )
}
fn screen(rect: Rect, x: f64, y: f64) -> (u16, u16) {
    // Match Canvas' 2×4 Braille subcell projection before converting to cells.
    // Rounding directly in character space can move a marker off its tracer.
    (
        rect.x + (((x + 180.) / 360. * (f64::from(rect.width) * 2. - 1.)) as u32 / 2) as u16,
        rect.y
            + (((NORTH - y) / (NORTH - SOUTH) * (f64::from(rect.height) * 4. - 1.)) as u32 / 4)
                as u16,
    )
}

pub fn draw(frame: &mut Frame, v: &View) {
    let t = v.theme;
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().fg(t.foreground).bg(t.background)),
        area,
    );
    if too_small(frame, t) {
        return;
    }
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Length(13),
        Constraint::Length(1),
    ])
    .split(area);
    let elapsed = v.model.started.elapsed().as_secs();
    let title = Line::from(vec![
        Span::styled(" GLOBAL NETWORK MAP ", Style::default().fg(t.accent).bold()),
        Span::styled(
            if v.demo {
                "  DEMO · SYNTHETIC TRAFFIC  "
            } else if v.stopped {
                "  STOPPED  "
            } else {
                "  LIVE  "
            },
            Style::default().fg(if v.demo || v.stopped {
                t.incoming
            } else {
                t.outgoing
            }),
        ),
        Span::styled(
            format!(
                " {}   ·   session {:02}:{:02}:{:02}",
                v.interface,
                elapsed / 3600,
                elapsed / 60 % 60,
                elapsed % 60
            ),
            Style::default().fg(t.muted),
        ),
    ]);
    frame.render_widget(Paragraph::new(title).block(block("", t)), layout[0]);
    let top = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[1]);
    for i in 0..2 {
        let color = if i == 0 { t.incoming } else { t.outgoing };
        let line = Line::from(vec![
            Span::styled(
                format!(" {}/s", bytes(v.model.total.rate[i])),
                Style::default().fg(color).bold(),
            ),
            Span::styled(
                format!(
                    "   ·   {} {}",
                    bytes(v.model.total.total[i] as f64),
                    if i == 0 { "received" } else { "sent" }
                ),
                Style::default().fg(t.muted),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(line).block(
                block(
                    if i == 0 {
                        " ↓ INBOUND / DOWNLOAD "
                    } else {
                        " ↑ OUTBOUND / UPLOAD "
                    },
                    t,
                )
                .title_style(Style::default().fg(color)),
            ),
            top[i],
        );
    }
    let nodes = nodes(v.model);
    let map_block = block(
        format!(
            " CONNECTION MAP   ·   {} active endpoints   ·   {} cities ",
            v.model.active().len(),
            nodes.len()
        ),
        t,
    );
    let map_area = map_block.inner(layout[2]);
    let origin = (v.origin.longitude, v.origin.latitude);
    let canvas = Canvas::default()
        .block(map_block)
        .marker(Marker::Braille)
        .background_color(t.background)
        .x_bounds([-180., 180.])
        .y_bounds([SOUTH, NORTH])
        .paint(|ctx| {
            let grid = faded(t.border, t.background, 0.38);
            for x in (-180..180).step_by(30) {
                ctx.draw(&CanvasLine {
                    x1: x as f64,
                    y1: SOUTH,
                    x2: x as f64,
                    y2: NORTH,
                    color: grid,
                });
            }
            for y in (-60..90).step_by(30) {
                ctx.draw(&CanvasLine {
                    x1: -180.,
                    y1: y as f64,
                    x2: 180.,
                    y2: y as f64,
                    color: grid,
                });
            }
            let coast_color = faded(t.accent, t.background, 0.55);
            for coast in v.coast {
                for pair in coast.windows(2) {
                    if (pair[0][0] - pair[1][0]).abs() < 180. {
                        ctx.draw(&CanvasLine {
                            x1: pair[0][0],
                            y1: pair[0][1],
                            x2: pair[1][0],
                            y2: pair[1][1],
                            color: coast_color,
                        });
                    }
                }
            }
            ctx.layer();
            for (index, node) in nodes.iter().take(128).enumerate() {
                for direction in 0..2 {
                    let Some(last) = node.last[direction] else {
                        continue;
                    };
                    let age = last.elapsed().as_secs_f64();
                    if age > 8. {
                        continue;
                    }
                    let color = if direction == 0 {
                        t.incoming
                    } else {
                        t.outgoing
                    };
                    let path_color = faded(color, t.background, if age < 2. { 0.48 } else { 0.22 });
                    let end = (node.x, node.y);
                    let mut prev = arc(origin, end, 0., direction);
                    for j in 1..=64 {
                        let p = arc(origin, end, j as f64 / 64., direction);
                        if (prev.0 - p.0).abs() < 180. {
                            ctx.draw(&CanvasLine {
                                x1: prev.0,
                                y1: prev.1,
                                x2: p.0,
                                y2: p.1,
                                color: path_color,
                            });
                        }
                        prev = p;
                    }
                    if age < 3. {
                        let phase = (v.elapsed * 0.42 + index as f64 * 0.137).fract();
                        for trail in 0..4 {
                            let time = (phase - trail as f64 * 0.035).rem_euclid(1.);
                            let p = arc(
                                origin,
                                end,
                                if direction == 0 { 1. - time } else { time },
                                direction,
                            );
                            ctx.draw(&Points {
                                coords: &[p],
                                color: faded(color, t.background, 1. - trail as f64 * 0.18),
                            });
                        }
                    }
                }
            }
            ctx.layer();
            for node in nodes.iter().take(128) {
                let color = if node.last[0] >= node.last[1] {
                    t.incoming
                } else {
                    t.outgoing
                };
                ctx.draw(&Points {
                    coords: &[(node.x, node.y)],
                    color,
                });
            }
        });
    frame.render_widget(canvas, layout[2]);
    // Labels use terminal cells so they remain readable and can avoid collisions.
    let mut occupied: Vec<Rect> = vec![];
    let (ox, oy) = screen(map_area, origin.0, origin.1.clamp(SOUTH, NORTH));
    let label = format!("◆ {}", v.origin.city);
    let w = (label.chars().count() as u16).min(map_area.width);
    let r = Rect::new(ox.min(map_area.right().saturating_sub(w)), oy, w, 1);
    frame.render_widget(
        Paragraph::new(label).style(Style::default().fg(t.accent).bg(t.background).bold()),
        r,
    );
    occupied.push(r);
    for node in nodes.iter().take(12) {
        if !(SOUTH..=NORTH).contains(&node.y) {
            continue;
        }
        let (x, y) = screen(map_area, node.x, node.y);
        let label = format!("• {}", node.city);
        let w = (label.chars().count() as u16).min(map_area.width);
        let rect = Rect::new(x.min(map_area.right().saturating_sub(w)), y, w, 1);
        if occupied
            .iter()
            .any(|r| r.intersects(Rect::new(rect.x.saturating_sub(1), rect.y, w + 2, 1)))
        {
            continue;
        }
        frame.render_widget(
            Paragraph::new(label).style(
                Style::default()
                    .fg(if node.last[0] >= node.last[1] {
                        t.incoming
                    } else {
                        t.outgoing
                    })
                    .bg(t.background),
            ),
            rect,
        );
        occupied.push(rect);
    }
    let bottom = Layout::horizontal([Constraint::Min(30), Constraint::Length(62)]).split(layout[3]);
    let left = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Min(3),
    ])
    .split(bottom[0]);
    for i in 0..2 {
        let history: Vec<_> = v.model.history[i].iter().copied().collect();
        let color = if i == 0 { t.incoming } else { t.outgoing };
        frame.render_widget(
            Sparkline::default()
                .data(&history)
                .style(Style::default().fg(color))
                .block(
                    block(
                        if i == 0 {
                            " ↓ DOWNLOAD · last 120 seconds "
                        } else {
                            " ↑ UPLOAD · last 120 seconds "
                        },
                        t,
                    )
                    .title_style(Style::default().fg(color)),
                ),
            left[i],
        );
    }
    let lines = vec![
        Line::from(format!(" Origin: {}, {}", v.origin.city, v.origin.country)),
        Line::from(Span::styled(
            format!(" {}  ·  IP bytes", v.database),
            Style::default().fg(t.muted),
        )),
        Line::from(Span::styled(
            if v.model.dropped > 0 {
                format!(
                    " Capture dropped {} packets · totals incomplete",
                    v.model.dropped
                )
            } else {
                format!(" {}", v.status)
            },
            Style::default().fg(if v.model.dropped > 0 || v.stopped {
                t.incoming
            } else {
                t.muted
            }),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), left[2]);
    let incoming = Style::default().fg(t.incoming);
    let outgoing = Style::default().fg(t.outgoing);
    let header = Row::new(vec![
        Cell::from("CONTINENT").style(Style::default().fg(t.muted)),
        Cell::from("↓ /s").style(incoming),
        Cell::from("↑ /s").style(outgoing),
        Cell::from("↓ TOTAL").style(incoming),
        Cell::from("↑ TOTAL").style(outgoing),
    ]);
    let rows = CONTINENTS.iter().enumerate().map(|(i, name)| {
        let c = &v.model.continents[i];
        Row::new(vec![
            Cell::from(*name),
            Cell::from(bytes(c.rate[0])).style(incoming),
            Cell::from(bytes(c.rate[1])).style(outgoing),
            Cell::from(bytes(c.total[0] as f64)).style(incoming),
            Cell::from(bytes(c.total[1] as f64)).style(outgoing),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(11),
            Constraint::Min(10),
            Constraint::Min(10),
            Constraint::Min(10),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(block(" TRAFFIC BY CONTINENT · session ", t));
    frame.render_widget(table, bottom[1]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if v.animation {
                    " q quit   ? help   o origin   space pause   "
                } else {
                    " q quit   ? help   o origin   space animate   "
                },
                Style::default().fg(t.muted),
            ),
            Span::styled("↓ inbound  ", incoming),
            Span::styled("↑ outbound  ", outgoing),
            Span::styled(format!(" · {}", t.source), Style::default().fg(t.muted)),
        ])),
        layout[4],
    );
    if let Some((mx, my)) = v.mouse {
        if map_area.contains(Position::new(mx, my)) {
            let hit = nodes
                .iter()
                .filter(|n| (SOUTH..=NORTH).contains(&n.y))
                .min_by_key(|n| {
                    let (x, y) = screen(map_area, n.x, n.y);
                    u32::from(x.abs_diff(mx)) + u32::from(y.abs_diff(my)) * 2
                });
            if let Some(n) = hit {
                let (x, y) = screen(map_area, n.x, n.y);
                if x.abs_diff(mx) <= 2 && y.abs_diff(my) <= 1 {
                    popup(frame, t, n, mx, my);
                }
            }
        }
    }
    if v.help {
        help(frame, t);
    }
}
fn popup(frame: &mut Frame, t: &Theme, n: &Node, mx: u16, my: u16) {
    let w = 48.min(frame.area().width);
    let h = 8;
    let rect = Rect::new(
        (mx + 2).min(frame.area().right() - w),
        (my + 1).min(frame.area().bottom() - h),
        w,
        h,
    );
    frame.render_widget(Clear, rect);
    let lines = vec![
        Line::from(Span::styled(
            format!("{}, {}", n.city, n.country),
            Style::default().fg(t.foreground).bold(),
        )),
        Line::from(format!(
            "{} active endpoint{} · approximate IP location",
            n.count,
            if n.count == 1 { "" } else { "s" }
        )),
        Line::from(Span::styled(
            format!(
                "↓ {}/s   ·   {}",
                bytes(n.rate[0]),
                bytes(n.total[0] as f64)
            ),
            Style::default().fg(t.incoming),
        )),
        Line::from(Span::styled(
            format!(
                "↑ {}/s   ·   {}",
                bytes(n.rate[1]),
                bytes(n.total[1] as f64)
            ),
            Style::default().fg(t.outgoing),
        )),
        Line::from(Span::styled(
            "Displayed endpoints · session bytes",
            Style::default().fg(t.muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(t.panel))
            .block(block(" CITY ", t)),
        rect,
    );
}
fn help(frame: &mut Frame, t: &Theme) {
    let rect = center(frame.area(), 86, 24);
    frame.render_widget(Clear, rect);
    let text="\n  q / Esc       Quit (Esc closes this panel first)\n  ?             Toggle help\n  o             Change map origin with offline city search\n  Space         Pause/resume path animation; capture continues\n  Mouse hover   Show city, country, rates and endpoint count\n\n  Red/theme red = inbound/download. Green/theme green = outbound/upload.\n  Rates update every second. Totals start when this session starts.\n  Counts are IP bytes, including IP headers and retransmissions.\n  Local = private/special addresses. Unknown = no country lookup.\n  Country-known traffic still counts when the city is unavailable.\n\n  VPN: the selected interface determines what is visible. A tunnel shows\n  inner destinations; a physical interface shows the encrypted VPN peer.\n  One interface avoids double-counting. Split-tunnel traffic on another\n  interface is outside this session. Restart after changing VPN/routes.\n\n  --list-interfaces / --interface NAME   Select capture interface\n  --update-db                           Refresh local DB-IP Lite data\n  --suggest-origin                      Opt-in public exit-IP suggestion\n\n  Data: Natural Earth (public domain); GeoNames and DB-IP (CC BY 4.0).";
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().bg(t.panel).fg(t.foreground))
            .block(block(" HELP / ABOUT ", t)),
        rect,
    );
}

pub struct Picker {
    pub query: String,
    pub selected: usize,
    pub matches: Vec<usize>,
    pub cities: Vec<City>,
    pub error: String,
}
impl Picker {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            matches: vec![],
            cities: geo::cities(),
            error: String::new(),
        }
    }
    pub fn search(&mut self) {
        self.matches = geo::search_cities(&self.cities, &self.query);
        self.selected = 0;
    }
    pub fn chosen(&self) -> Option<Origin> {
        self.matches
            .get(self.selected)
            .map(|i| self.cities[*i].origin.clone())
    }
    pub fn draw(&self, frame: &mut Frame, t: &Theme) {
        frame.render_widget(
            Block::default().style(Style::default().bg(t.background).fg(t.foreground)),
            frame.area(),
        );
        if frame.area().width < 70 || frame.area().height < 23 {
            frame.render_widget(
                Paragraph::new("Resize to 70 × 23 for city selection. Esc cancels."),
                frame.area(),
            );
            return;
        }
        let rect = center(frame.area(), 90, 25);
        let inner = block(" SET YOUR MAP ORIGIN ", t).inner(rect);
        frame.render_widget(block(" SET YOUR MAP ORIGIN ", t), rect);
        let layout = Layout::vertical([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(4),
        ])
        .split(inner);
        frame.render_widget(Paragraph::new("\n Choose your physical city, independently of your VPN exit.\n Search locally by city and country name or code (e.g. Paris, FR).").style(Style::default().fg(t.muted)),layout[0]);
        frame.render_widget(
            Paragraph::new(format!(" {}", self.query))
                .style(Style::default().fg(t.accent))
                .block(block(" CITY / COUNTRY ", t)),
            layout[1],
        );
        let items: Vec<_> = self
            .matches
            .iter()
            .map(|i| {
                let c = &self.cities[*i];
                ListItem::new(format!(
                    " {} · {} ({}, {})   {:.2}, {:.2}",
                    c.origin.city,
                    c.origin.country,
                    c.code,
                    c.region,
                    c.origin.latitude,
                    c.origin.longitude
                ))
            })
            .collect();
        let mut state = ListState::default().with_selected(if self.matches.is_empty() {
            None
        } else {
            Some(self.selected)
        });
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(Style::default().bg(t.panel).fg(t.accent).bold())
                .highlight_symbol("›"),
            layout[2],
            &mut state,
        );
        frame.render_widget(Paragraph::new(format!(" {}\n ↑/↓ select   Enter save   Esc cancel\n Smaller town? Use --origin 'City,Country,latitude,longitude'.",if self.error.is_empty(){"Coordinates stay local. External-IP lookup is optional: --suggest-origin"}else{&self.error})).style(Style::default().fg(t.muted)),layout[3]);
        frame.set_cursor_position((
            layout[1].x
                + 2
                + (self.query.chars().count() as u16).min(layout[1].width.saturating_sub(4)),
            layout[1].y + 1,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    #[test]
    fn marker_hitboxes_match_canvas_coordinates() {
        use ratatui::widgets::canvas::{Context, Painter};
        let rect = Rect::new(1, 7, 158, 26);
        let mut context = Context::new(
            rect.width,
            rect.height,
            [-180., 180.],
            [SOUTH, NORTH],
            Marker::Braille,
        );
        let painter = Painter::from(&mut context);
        for (x, y) in [
            (-112.074, 33.4484),
            (139.69, 35.68),
            (-180., SOUTH),
            (180., NORTH),
        ] {
            let (px, py) = painter.get_point(x, y).unwrap();
            assert_eq!(
                screen(rect, x, y),
                (rect.x + (px / 2) as u16, rect.y + (py / 4) as u16)
            );
        }
    }
    #[test]
    fn dateline_arc_takes_short_route() {
        let middle = arc((170., 20.), (-170., 25.), 0.5, 0);
        assert!(middle.0.abs() > 170.);
        for i in 0..100 {
            let (_, lat) = arc((0., 80.), (150., 80.), i as f64 / 100., 0);
            assert!(lat < NORTH);
        }
    }
    #[test]
    fn minimum_layout_and_resize_screen() {
        let theme = Theme::default();
        let model = Model::default();
        let coast = coastline();
        let origin = Origin {
            city: "Test City".into(),
            country: "Test Country".into(),
            latitude: 33.,
            longitude: -112.,
        };
        let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &View {
                        model: &model,
                        origin: &origin,
                        theme: &theme,
                        coast: &coast,
                        interface: "test0",
                        database: "Offline",
                        status: "Ready",
                        demo: true,
                        stopped: false,
                        animation: true,
                        help: false,
                        mouse: None,
                        elapsed: 0.,
                    },
                )
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        for label in [
            "TRAFFIC BY CONTINENT",
            "N. America",
            "Unknown",
            "DEMO",
            "DOWNLOAD",
            "UPLOAD",
        ] {
            assert!(text.contains(label), "{label}");
        }
        let mut small = Terminal::new(TestBackend::new(60, 20)).unwrap();
        small
            .draw(|f| {
                assert!(too_small(f, &theme));
            })
            .unwrap();
        assert!(small
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains("Resize your terminal"));
    }
}

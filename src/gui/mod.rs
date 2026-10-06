mod chart;
mod map;
mod palette;
mod settings;

use crate::Args;
use anyhow::Result;
use chart::TrafficChart;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, RichText, Stroke, StrokeKind};
use global_network_map::{
    config::{Config, Origin},
    demo,
    geo::{GeoDb, CONTINENTS},
    model::{bytes, Model},
    network::{self, Capture, Interface},
};
use map::WorldMap;
use palette::{mix, Palette};
use settings::{Draft, JobResult};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Ready,
    Demo,
    Authorizing,
    Live,
    Stopped,
}

pub struct NetworkMapApp {
    args: Args,
    config: Config,
    db: Arc<GeoDb>,
    model: Model,
    capture: Option<Capture>,
    mode: Mode,
    synthetic: bool,
    status: String,
    active_interface: String,
    interfaces: Vec<Interface>,
    pub palette: Palette,
    world: WorldMap,
    chart: TrafficChart,
    opened: Instant,
    theme_checked: Instant,
    demo_tick: Instant,
    sample_tick: Instant,
    heartbeat: Option<Instant>,
    demo_step: u64,
    animation: bool,
    phase: f32,
    last_frame: Instant,
    settings_open: bool,
    draft: Draft,
    about_open: bool,
    job: Option<mpsc::Receiver<JobResult>>,
    job_status: String,
    screenshot_requested: bool,
    screenshot_done: bool,
    frames: u64,
}
impl NetworkMapApp {
    pub fn new(args: Args) -> Result<Self> {
        let config = Config::load()?;
        let db = Arc::new(GeoDb::open(
            args.database.as_deref().or(config.database.as_deref()),
        )?);
        let draft = Draft::new(&config);
        let interfaces = network::interfaces().unwrap_or_default();
        let settings_open = config.origin.is_none() && !args.demo;
        let now = Instant::now();
        let autostart = args.start;
        let mut app = Self {
            args,
            config,
            db,
            model: Model::default(),
            capture: None,
            mode: Mode::Ready,
            synthetic: false,
            status: "Choose an origin and start capture. All location lookups stay local.".into(),
            active_interface: String::new(),
            interfaces,
            palette: Palette::load(),
            world: WorldMap::load()?,
            chart: TrafficChart::default(),
            opened: now,
            theme_checked: now,
            demo_tick: now,
            sample_tick: now,
            heartbeat: None,
            demo_step: 0,
            animation: true,
            phase: 0.,
            last_frame: now,
            settings_open,
            draft,
            about_open: false,
            job: None,
            job_status: String::new(),
            screenshot_requested: false,
            screenshot_done: false,
            frames: 0,
        };
        if app.args.demo {
            app.start_demo();
        } else if autostart {
            app.start_capture();
        }
        Ok(app)
    }
    pub fn setup_fonts(&mut self, ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        // Use the machine's familiar UI font when available; embedded fonts remain the fallback.
        for path in [
            "/usr/share/fonts/TTF/Inter-Regular.ttf",
            "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("desktop".into(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, "desktop".into());
                break;
            }
        }
        ctx.set_fonts(fonts);
    }
    fn start_demo(&mut self) {
        self.capture.take();
        self.synthetic = true;
        self.mode = Mode::Demo;
        self.heartbeat = None;
        let now = Instant::now();
        let start = now - Duration::from_secs(60);
        self.model = Model::starting_at(start);
        self.chart = TrafficChart::default();
        for second in 0..60 {
            for part in 0..5 {
                let step = second * 5 + part;
                demo::step_at(
                    &mut self.model,
                    &self.db,
                    step,
                    start + Duration::from_millis(step * 200),
                );
            }
            self.model.tick_at(start + Duration::from_secs(second + 1));
            self.chart.push((second + 1) as f64, self.model.total.rate);
        }
        self.demo_step = 300;
        self.demo_tick = now;
        self.sample_tick = now;
        self.phase = 0.;
        self.active_interface = "Synthetic traffic".into();
        self.status = "Demo data only · no packets are being captured".into();
    }
    fn start_capture(&mut self) {
        if self.config.origin.is_none() {
            self.settings_open = true;
            self.status = "Choose your map origin before starting live capture.".into();
            return;
        }
        self.capture.take();
        self.synthetic = false;
        self.model = Model::default();
        self.chart = TrafficChart::default();
        self.heartbeat = None;
        self.sample_tick = Instant::now();
        let result = (|| -> Result<(Capture, String)> {
            self.db = Arc::new(GeoDb::open(
                self.args
                    .database
                    .as_deref()
                    .or(self.config.database.as_deref()),
            )?);
            self.interfaces = network::interfaces()?;
            let interface = network::choose_interface(
                &self.interfaces,
                self.args
                    .interface
                    .as_deref()
                    .or(self.config.interface.as_deref()),
            )?;
            let label = format!(
                "{}{}",
                interface.name,
                if interface.tunnel {
                    " · VPN tunnel"
                } else {
                    ""
                }
            );
            Ok((Capture::start_graphical(&interface.name)?, label))
        })();
        match result {
            Ok((capture, interface)) => {
                self.capture = Some(capture);
                self.active_interface = interface;
                self.mode = Mode::Authorizing;
                self.status = "Complete the desktop authentication prompt to begin capture.".into();
            }
            Err(e) => {
                self.mode = Mode::Stopped;
                self.status = format!("Capture could not start: {e:#}");
            }
        }
    }
    fn stop(&mut self) {
        self.capture.take();
        self.mode = Mode::Stopped;
        self.status = if self.synthetic {
            "Demo stopped · displayed totals are synthetic"
        } else {
            "Capture stopped · session totals retained"
        }
        .into();
    }
    fn pump(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.animation {
            self.phase += now.duration_since(self.last_frame).as_secs_f32();
        }
        self.last_frame = now;
        if self.mode == Mode::Demo
            && now.duration_since(self.demo_tick) >= Duration::from_millis(200)
        {
            demo::step(&mut self.model, &self.db, self.demo_step);
            self.demo_step += 1;
            self.demo_tick = now;
        }
        let mut failure = None;
        if let Some(capture) = self.capture.as_mut() {
            for _ in 0..32 {
                match capture.receiver.try_recv() {
                    Ok(batch) => {
                        self.model.dropped = batch.dropped;
                        self.heartbeat = Some(now);
                        if self.mode == Mode::Authorizing {
                            self.mode = Mode::Live;
                            self.status="Local capture active · IP bytes, including headers and retransmissions".into();
                        }
                        for traffic in batch.traffic {
                            self.model.ingest(traffic, &self.db);
                        }
                    }
                    Err(_) => break,
                }
            }
            failure = capture.exited();
        }
        if let Some(error) = failure {
            self.capture.take();
            self.mode = Mode::Stopped;
            self.status = error;
        }
        self.model.tick_at(now);
        if now.duration_since(self.sample_tick) >= Duration::from_secs(1) {
            if self.mode == Mode::Live || self.mode == Mode::Demo || self.mode == Mode::Stopped {
                self.chart.push(
                    self.model.started.elapsed().as_secs_f64(),
                    self.model.total.rate,
                );
            }
            self.sample_tick = now;
        }
        if now.duration_since(self.theme_checked) >= Duration::from_secs(1) {
            let palette = Palette::load();
            if palette != self.palette {
                palette.apply(ctx);
                self.palette = palette;
            }
            self.theme_checked = now;
        }
        self.poll_job();
    }
    fn origin(&self) -> Option<Origin> {
        self.config
            .origin
            .clone()
            .or_else(|| self.synthetic.then(demo::origin))
    }
    fn badge(&self) -> (&str, Color32) {
        if self.synthetic {
            return (
                if self.mode == Mode::Demo {
                    "DEMO · SYNTHETIC"
                } else {
                    "DEMO · STOPPED"
                },
                self.palette.incoming,
            );
        }
        match self.mode {
            Mode::Live
                if self
                    .heartbeat
                    .is_some_and(|t| t.elapsed() > Duration::from_secs(3)) =>
            {
                ("NO UPDATES", self.palette.incoming)
            }
            Mode::Live => ("LIVE", self.palette.outgoing),
            Mode::Authorizing => ("AUTHENTICATING", self.palette.accent),
            Mode::Stopped => ("STOPPED", self.palette.incoming),
            _ => ("READY", self.palette.muted),
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("GLOBAL NETWORK MAP")
                    .size(18.)
                    .strong()
                    .color(self.palette.accent),
            );
            ui.add_space(8.);
            let (badge, color) = self.badge();
            ui.label(RichText::new(badge).monospace().size(18.).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.draft = Draft::new(&self.config);
                    self.settings_open = true;
                }
                if ui.button("About").clicked() {
                    self.about_open = true;
                }
                if matches!(self.mode, Mode::Live | Mode::Authorizing) {
                    if ui.button("Stop capture").clicked() {
                        self.stop();
                    }
                } else if ui
                    .button(RichText::new("Start capture").color(self.palette.outgoing))
                    .clicked()
                {
                    self.start_capture();
                }
                if self.mode == Mode::Demo {
                    if ui.button("Stop demo").clicked() {
                        self.stop();
                    }
                } else if ui.button("Demo").clicked() {
                    self.start_demo();
                }
            });
        });
    }
    fn metrics(&self, ui: &mut egui::Ui, rect: Rect) {
        let gap = 12.;
        let w = (rect.width() - 2. * gap) / 3.;
        for i in 0..3 {
            let r = Rect::from_min_size(
                rect.min + vec2(i as f32 * (w + gap), 0.),
                vec2(w, rect.height()),
            );
            let painter = ui.painter();
            let p = &self.palette;
            painter.rect_filled(r, 8., mix(p.bg, p.panel, 0.5));
            painter.rect_stroke(r, 8., Stroke::new(1.0_f32, p.border), StrokeKind::Inside);
            let (label, value, detail, color) = match i {
                0 => (
                    "↓ DOWNLOAD",
                    format!("{}/s", bytes(self.model.total.rate[0])),
                    format!(
                        "{} received this session",
                        bytes(self.model.total.total[0] as f64)
                    ),
                    p.incoming,
                ),
                1 => (
                    "↑ UPLOAD",
                    format!("{}/s", bytes(self.model.total.rate[1])),
                    format!(
                        "{} sent this session",
                        bytes(self.model.total.total[1] as f64)
                    ),
                    p.outgoing,
                ),
                _ => {
                    let elapsed = self.model.started.elapsed().as_secs();
                    (
                        "ACTIVE ENDPOINTS",
                        self.model.active().len().to_string(),
                        format!(
                            "{}  ·  {:02}:{:02}:{:02}",
                            if self.active_interface.is_empty() {
                                "No capture"
                            } else {
                                &self.active_interface
                            },
                            elapsed / 3600,
                            elapsed / 60 % 60,
                            elapsed % 60
                        ),
                        p.accent,
                    )
                }
            };
            painter.text(
                r.min + vec2(16., 12.),
                Align2::LEFT_TOP,
                label,
                FontId::monospace(18.),
                color,
            );
            painter.text(
                r.min + vec2(16., 39.),
                Align2::LEFT_TOP,
                value,
                FontId::proportional(27.),
                color,
            );
            painter.text(
                r.min + vec2(16., 77.),
                Align2::LEFT_TOP,
                detail,
                FontId::proportional(18.),
                p.muted,
            );
        }
    }
    fn continents(&self, ui: &mut egui::Ui, rect: Rect) {
        let p = &self.palette;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10., mix(p.bg, p.panel, 0.38));
        painter.rect_stroke(
            rect,
            10.,
            Stroke::new(1.0_f32, p.border),
            StrokeKind::Inside,
        );
        painter.text(
            rect.min + vec2(16., 15.),
            Align2::LEFT_TOP,
            "TRAFFIC BY CONTINENT",
            FontId::monospace(14.),
            p.accent,
        );
        let content = Rect::from_min_max(rect.min + vec2(14., 50.), rect.max - vec2(14., 12.));
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.style_mut().override_font_id = Some(FontId::proportional(14.));
            egui::ScrollArea::vertical()
                .id_salt("continents")
                .show(ui, |ui| {
                    for (i, name) in CONTINENTS.iter().enumerate() {
                        let c = &self.model.continents[i];
                        ui.strong(*name);
                        for (direction, arrow, color) in
                            [(0, "↓", p.incoming), (1, "↑", p.outgoing)]
                        {
                            ui.colored_label(
                                color,
                                format!(
                                    "{arrow} {}/s  ·  {} total",
                                    bytes(c.rate[direction]),
                                    bytes(c.total[direction] as f64)
                                ),
                            );
                        }
                        ui.separator();
                    }
                });
        });
    }
    fn dashboard(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(self.palette.bg)
                    .inner_margin(egui::Margin::symmetric(20, 14)),
            )
            .show(ctx, |ui| self.header(ui));
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(self.palette.bg)
                    .inner_margin(egui::Margin::symmetric(20, 9)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let error = self.mode == Mode::Stopped || self.model.dropped > 0;
                    let text = if self.model.dropped > 0 {
                        format!(
                            "{} packets dropped · totals may undercount",
                            self.model.dropped
                        )
                    } else {
                        self.status.clone()
                    };
                    ui.label(RichText::new(text).size(18.).color(if error {
                        self.palette.incoming
                    } else {
                        self.palette.muted
                    }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(&self.palette.source)
                                .size(18.)
                                .color(self.palette.muted),
                        );
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.palette.bg)
                    .inner_margin(egui::Margin::symmetric(20, 4)),
            )
            .show(ctx, |ui| {
                let area = ui.max_rect();
                if area.width() < 860. || area.height() < 580. {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space((area.height() * 0.4).max(0.));
                            ui.label(
                                "Enlarge the window to at least 960 × 720 to view the dashboard.",
                            );
                            if ui.button("Use fullscreen (F11)").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                            }
                        });
                    });
                    return;
                }
                let metrics = Rect::from_min_size(area.min, vec2(area.width(), 108.));
                self.metrics(ui, metrics);
                let bottom_h = (area.height() * 0.24).clamp(210., 260.);
                let table_w = (area.width() * 0.27).clamp(350., 420.);
                let map_rect = Rect::from_min_max(
                    pos2(area.left(), metrics.bottom() + 12.),
                    pos2(area.right() - table_w - 12., area.bottom() - bottom_h - 12.),
                );
                let nodes = map::city_nodes(&self.model);
                let origin = self.origin();
                self.world.show(
                    ui,
                    map_rect,
                    &self.palette,
                    &nodes,
                    origin.as_ref(),
                    self.phase,
                );
                let controls =
                    Rect::from_min_size(map_rect.left_bottom() + vec2(16., -63.), vec2(340., 40.));
                ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(if self.animation {
                                "Pause paths"
                            } else {
                                "Animate paths"
                            })
                            .clicked()
                        {
                            self.animation = !self.animation;
                        }
                        if ui.small_button("Fit world").clicked() {
                            self.world.reset();
                        }
                    });
                });
                let chart_rect = Rect::from_min_max(
                    pos2(area.left(), area.bottom() - bottom_h),
                    pos2(area.right(), area.bottom()),
                );
                let table_rect = Rect::from_min_max(
                    pos2(map_rect.right() + 12., map_rect.top()),
                    pos2(area.right(), map_rect.bottom()),
                );
                self.chart.show(
                    ui,
                    chart_rect,
                    &self.palette,
                    self.model.started.elapsed().as_secs_f64(),
                );
                self.continents(ui, table_rect);
            });
        if self.settings_open {
            self.settings(ctx);
        }
        if self.about_open {
            egui::Window::new("About Global Network Map").open(&mut self.about_open).resizable(false).default_width(520.).show(ctx,|ui| {
                ui.heading("Your network, around the world");
                ui.label("Only this computer's traffic on one selected interface is counted. Session totals reset when capture restarts.");
                ui.separator();ui.label("Rates include IP headers and retransmissions. Local = private/special addresses. Unknown = no country match. City locations are approximate; paths illustrate direction, not physical network routes.");
                ui.label("For a VPN, choose its tunnel interface to see inner destinations. A physical interface usually shows the encrypted VPN peer. Split-tunnel traffic on other interfaces is outside this session.");
                ui.separator();ui.label(format!("Map: Natural Earth 1:50m · {} countries · {} vertices (public domain)",self.world.countries,self.world.vertices));
                ui.hyperlink_to("Natural Earth","https://www.naturalearthdata.com/");
                ui.hyperlink_to("IP Geolocation by DB-IP · CC BY 4.0","https://db-ip.com/");
                ui.hyperlink_to("City search and continent data: GeoNames · CC BY 4.0","https://www.geonames.org/");
                ui.label("Normal monitoring makes no geolocation requests. Database updates and public-IP suggestions are explicit actions in Settings.");
                ui.small("Press Escape to close preferences/about. Use the window close button to exit.");
            });
        }
    }
}
impl eframe::App for NetworkMapApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frames += 1;
        // Reissue after the surface is mapped: some Wayland compositors discard
        // the fullscreen hint on the initial window-creation request.
        if self.frames == 2 && (self.args.fullscreen || self.args.screenshot.is_some()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        self.pump(ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.settings_open = false;
            self.about_open = false;
        }
        self.dashboard(ctx);
        if self.args.screenshot.is_some()
            && !self.screenshot_requested
            && self.opened.elapsed() > Duration::from_secs(3)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot_requested = true;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = &self.args.screenshot {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    match image::save_buffer(
                        path,
                        &rgba,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => {
                            eprintln!("Saved graphical preview: {}", path.display());
                            self.screenshot_done = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(e) => {
                            self.status = format!("Screenshot failed: {e}");
                            self.args.screenshot = None;
                        }
                    }
                }
            }
        }
        if self
            .args
            .quit_after
            .is_some_and(|s| self.opened.elapsed() >= Duration::from_secs(s))
        {
            if self.args.screenshot.is_some() && !self.screenshot_done {
                eprintln!("Screenshot was not completed before the time limit.");
            }
            eprintln!("Launch check: {:?}; synthetic={}; received={} bytes; sent={} bytes; dropped={} packets; {}",
                self.mode, self.synthetic, self.model.total.total[0], self.model.total.total[1], self.model.dropped, self.status);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(if self.animation { 33 } else { 100 }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopping_demo_preserves_synthetic_label_and_totals() {
        let mut app = NetworkMapApp::new(Args {
            demo: true,
            fullscreen: false,
            interface: None,
            start: false,
            database: None,
            screenshot: None,
            quit_after: None,
        })
        .unwrap();
        let total = app.model.total.total;
        assert!(total[0] > 0 && total[1] > 0);
        app.stop();
        assert_eq!(app.badge().0, "DEMO · STOPPED");
        assert_eq!(app.model.total.total, total);
        assert!(app.capture.is_none());
        let ctx = egui::Context::default();
        for size in [vec2(640., 480.), vec2(960., 720.), vec2(1440., 960.)] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), size)),
                    ..Default::default()
                },
                |ctx| app.dashboard(ctx),
            );
            assert!(!output.shapes.is_empty());
        }
        // Ready for live mode: demo history must be cleared even when interface selection fails.
        app.config.origin = Some(demo::origin());
        app.args.interface = Some("nonexistent-test-interface".into());
        app.start_capture();
        assert!(!app.synthetic);
        assert_eq!(app.badge().0, "STOPPED");
        assert_eq!(app.model.total.total, [0, 0]);
        assert!(app.chart.samples.is_empty());
    }
}

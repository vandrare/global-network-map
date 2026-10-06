use global_network_map::{
    demo::{origin as demo_origin, step as demo_step},
    network,
};
mod theme;
mod ui;

use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use global_network_map::{
    config::{Config, Origin},
    geo::GeoDb,
    model::Model,
};
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    style::Color,
    Terminal,
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "A live, Omarchy-aware world map of this computer's IP traffic"
)]
struct Args {
    /// Preview explicitly synthetic traffic without capture privileges or a database
    #[arg(long)]
    demo: bool,
    /// Capture exactly this interface (automatic selection follows the Internet route)
    #[arg(short, long)]
    interface: Option<String>,
    /// List available interfaces and their addresses
    #[arg(long)]
    list_interfaces: bool,
    /// Choose and save your physical city using offline search
    #[arg(long)]
    setup: bool,
    /// Configure an exact origin: 'City,Country,latitude,longitude'
    #[arg(long, allow_hyphen_values = true)]
    origin: Option<String>,
    /// Use an existing local city MMDB instead of the downloaded DB-IP databases
    #[arg(long)]
    database: Option<PathBuf>,
    /// Explicitly download and verify fresh IPv4 + IPv6 DB-IP Lite databases
    #[arg(long)]
    update_db: bool,
    /// Contact ipify for the public exit IP and suggest a city using the local database
    #[arg(long)]
    suggest_origin: bool,
    /// Render a deterministic demo to an SVG file without opening a terminal
    #[arg(long, value_name = "FILE.svg")]
    snapshot: Option<PathBuf>,
    /// Capture for N seconds and print a JSON summary without opening the UI
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..=3600))]
    headless_seconds: Option<u64>,
}

type Term = Terminal<CrosstermBackend<io::Stdout>>;
struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<(Self, Term)> {
        enable_raw_mode()?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        terminal.hide_cursor()?;
        Ok((guard, terminal))
    }
}
fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

fn parse_origin(text: &str) -> Result<Origin> {
    let fields: Vec<_> = text.split(',').map(str::trim).collect();
    if fields.len() != 4 {
        bail!("Use --origin 'City,Country,latitude,longitude'");
    }
    let origin = Origin {
        city: fields[0].into(),
        country: fields[1].into(),
        latitude: fields[2].parse().context("Invalid latitude")?,
        longitude: fields[3].parse().context("Invalid longitude")?,
    };
    origin.validate()?;
    Ok(origin)
}
fn picker_key(picker: &mut ui::Picker, code: KeyCode, config: &mut Config) -> Result<Option<bool>> {
    match code {
        KeyCode::Esc => return Ok(Some(false)),
        KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
        KeyCode::Down => {
            picker.selected = (picker.selected + 1).min(picker.matches.len().saturating_sub(1))
        }
        KeyCode::Enter => {
            if let Some(origin) = picker.chosen() {
                let mut updated = config.clone();
                updated.origin = Some(origin);
                match updated.save() {
                    Ok(()) => {
                        *config = updated;
                        return Ok(Some(true));
                    }
                    Err(e) => picker.error = format!("Could not save: {e}"),
                }
            }
        }
        KeyCode::Backspace => {
            picker.query.pop();
            picker.search();
        }
        KeyCode::Char(c) if picker.query.len() < 100 => {
            picker.query.push(c);
            picker.search();
        }
        _ => {}
    }
    Ok(None)
}
fn setup(config: &mut Config, stop: &AtomicBool) -> Result<bool> {
    let (_guard, mut terminal) = TerminalGuard::enter()?;
    let theme = theme::Theme::load();
    let mut picker = ui::Picker::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        terminal.draw(|f| picker.draw(f, &theme))?;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return Ok(false);
                }
                if let Some(done) = picker_key(&mut picker, key.code, config)? {
                    return Ok(done);
                }
            }
        }
    }
}

fn drain(capture: &network::Capture, model: &mut Model, db: &GeoDb) {
    // Bound work per frame so a busy capture cannot starve input and repaint.
    for _ in 0..64 {
        match capture.receiver.try_recv() {
            Ok(batch) => {
                model.dropped = batch.dropped;
                for traffic in batch.traffic {
                    model.ingest(traffic, db);
                }
            }
            Err(_) => break,
        }
    }
}
fn headless(
    seconds: u64,
    capture: &mut network::Capture,
    db: &GeoDb,
    stop: &AtomicBool,
) -> Result<()> {
    let mut model = Model::default();
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        drain(capture, &mut model, db);
        model.tick();
        if let Some(error) = capture.exited() {
            bail!("{error}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drain(capture, &mut model, db);
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"received":model.total.total[0],"sent":model.total.total[1],"endpoints":model.endpoints.len(),"dropped_packets":model.dropped,
        "continents":global_network_map::geo::CONTINENTS.iter().enumerate().map(|(i,name)|serde_json::json!({"name":name,"received":model.continents[i].total[0],"sent":model.continents[i].total[1]})).collect::<Vec<_>>()})
        )?
    );
    Ok(())
}

fn run_ui(
    args: &Args,
    config: &mut Config,
    db: &GeoDb,
    mut capture: Option<network::Capture>,
    interface: &str,
    stop: &AtomicBool,
) -> Result<()> {
    let (_guard, mut terminal) = TerminalGuard::enter()?;
    let mut theme = theme::Theme::load();
    let coast = ui::coastline();
    let mut model = Model::default();
    let start = Instant::now();
    let mut theme_check = start;
    let mut demo_tick = start - Duration::from_secs(1);
    let mut step = 0;
    let mut mouse = None;
    let mut help = false;
    let mut animation = true;
    let mut animation_time = 0.0;
    let mut last_frame = Instant::now();
    let mut picker: Option<ui::Picker> = None;
    let mut status = if args.demo {
        "Synthetic preview · no network capture".into()
    } else {
        "Single interface · hover a city for details".into()
    };
    let mut capture_error = None;
    loop {
        let now = Instant::now();
        if animation {
            animation_time += now.duration_since(last_frame).as_secs_f64();
        }
        last_frame = now;
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if args.demo && demo_tick.elapsed() >= Duration::from_millis(200) {
            demo_step(&mut model, db, step);
            step += 1;
            demo_tick = Instant::now();
        }
        if let Some(c) = capture.as_mut() {
            drain(c, &mut model, db);
            if capture_error.is_none() {
                if let Some(e) = c.exited() {
                    status = e.clone();
                    capture_error = Some(e);
                }
            }
        }
        model.tick();
        if theme_check.elapsed() >= Duration::from_secs(1) {
            theme = theme::Theme::load();
            theme_check = Instant::now();
        }
        let default_origin = demo_origin();
        let origin = config.origin.as_ref().unwrap_or(&default_origin);
        terminal.draw(|f| {
            if let Some(p) = &picker {
                p.draw(f, &theme);
            } else {
                ui::draw(
                    f,
                    &ui::View {
                        model: &model,
                        origin,
                        theme: &theme,
                        coast: &coast,
                        interface,
                        database: &db.status,
                        status: &status,
                        demo: args.demo,
                        stopped: capture_error.is_some(),
                        animation,
                        help,
                        mouse,
                        elapsed: animation_time,
                    },
                );
            }
        })?;
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        break;
                    }
                    if let Some(p) = &mut picker {
                        if picker_key(p, key.code, config)?.is_some() {
                            picker = None;
                        }
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Esc => {
                            if help {
                                help = false;
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('?') => help = !help,
                        KeyCode::Char(' ') => animation = !animation,
                        KeyCode::Char('o') => {
                            picker = Some(ui::Picker::new());
                            mouse = None;
                        }
                        _ => {}
                    }
                }
                Event::Mouse(event) => mouse = Some((event.column, event.row)),
                Event::Resize(_, _) => mouse = None,
                _ => {}
            }
        }
    }
    drop(terminal);
    drop(_guard);
    drop(capture);
    if let Some(error) = capture_error {
        bail!("{error}");
    }
    Ok(())
}

fn snapshot(path: &std::path::Path, origin: &Origin, db: &GeoDb) -> Result<()> {
    let mut model = Model::default();
    let theme = theme::Theme::load();
    let coast = ui::coastline();
    for step in 0..120 {
        demo_step(&mut model, db, step);
    }
    // A fixed elapsed interval produces reproducible example rates and totals.
    let tick = Instant::now() + Duration::from_secs(24);
    model.tick_at(tick);
    model.started = Instant::now() - Duration::from_secs(24);
    for i in 0..2 {
        model.history[i] = (0..100)
            .map(|n| ((n as f64 * 0.12 + i as f64).sin().abs() * model.total.rate[i]) as u64)
            .collect();
    }
    let mut terminal = Terminal::new(TestBackend::new(160, 48))?;
    terminal.draw(|f| {
        ui::draw(
            f,
            &ui::View {
                model: &model,
                origin,
                theme: &theme,
                coast: &coast,
                interface: "demo0",
                database: "DB-IP Lite · offline IPv4 + IPv6",
                status: "Synthetic preview · no network capture",
                demo: true,
                stopped: false,
                animation: true,
                help: false,
                mouse: None,
                elapsed: 1.4,
            },
        )
    })?;
    let color = |c: Color, background: bool| match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Red => "#f07f91".into(),
        Color::Green => "#85cfbb".into(),
        Color::Cyan => "#6cb6ff".into(),
        Color::DarkGray => "#7289a6".into(),
        _ => {
            if background {
                "#071426".into()
            } else {
                "#d6e6f5".into()
            }
        }
    };
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut svg=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1600\" height=\"960\" viewBox=\"0 0 1600 960\"><rect width=\"100%\" height=\"100%\" fill=\"{}\"/><g font-family=\"JetBrains Mono,monospace\" font-size=\"16\">",color(theme.background,true));
    for y in 0..48 {
        for x in 0..160 {
            let c = &terminal.backend().buffer()[(x, y)];
            if c.symbol() != " " {
                svg.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>",
                    x * 10,
                    y * 20 + 16,
                    color(c.fg, false),
                    escape(c.symbol())
                ));
            }
        }
    }
    svg.push_str("</g></svg>");
    std::fs::write(path, svg)?;
    println!("Demo snapshot: {}", path.display());
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    if unsafe { libc::geteuid() } == 0 {
        bail!(
            "Run the interface as your normal user; it invokes sudo only for the capture helper."
        );
    }
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, stop.clone())?;
    }
    if args.update_db {
        return network::update_db();
    }
    if args.list_interfaces {
        let list = network::interfaces()?;
        let selected = network::choose_interface(&list, None).ok();
        for d in list {
            println!(
                "{} {:16} {:4} {:8} {}",
                if selected.as_ref().is_some_and(|s| s.name == d.name) {
                    "*"
                } else {
                    " "
                },
                d.name,
                if d.up { "up" } else { "down" },
                if d.tunnel { "VPN/tun" } else { "physical" },
                d.addresses
            );
        }
        println!("* Automatic selection. Override with --interface NAME. One interface is captured per session.");
        return Ok(());
    }
    let mut config = Config::load()?;
    if let Some(origin) = &args.origin {
        config.origin = Some(parse_origin(origin)?);
        config.save()?;
    }
    let db = GeoDb::open(args.database.as_deref().or(config.database.as_deref()))?;
    if args.suggest_origin {
        return network::suggest_origin(&db);
    }
    if let Some(path) = &args.snapshot {
        return snapshot(path, config.origin.as_ref().unwrap_or(&demo_origin()), &db);
    }
    if args.setup && args.origin.is_some() {
        return Ok(());
    }
    if args.headless_seconds.is_none()
        && (!io::stdout().is_terminal() || !io::stdin().is_terminal())
    {
        bail!("Open this app in Kitty, Alacritty or Ghostty. Use --snapshot FILE.svg for a noninteractive preview.");
    }
    if args.setup {
        if args.origin.is_none() {
            setup(&mut config, &stop)?;
        }
        return Ok(());
    }
    if !args.demo
        && args.headless_seconds.is_none()
        && config.origin.is_none()
        && !setup(&mut config, &stop)?
    {
        return Ok(());
    }
    let (capture, interface) = if args.demo {
        (None, "demo0 · synthetic".into())
    } else {
        let list = network::interfaces()?;
        let selected = network::choose_interface(
            &list,
            args.interface.as_deref().or(config.interface.as_deref()),
        )?;
        let label = format!(
            "{}{}",
            selected.name,
            if selected.tunnel {
                " · VPN/tunnel"
            } else {
                ""
            }
        );
        (Some(network::Capture::start(&selected.name)?), label)
    };
    if let Some(seconds) = args.headless_seconds {
        let mut capture =
            capture.context("--headless-seconds requires live capture, not --demo")?;
        return headless(seconds, &mut capture, &db, &stop);
    }
    let old = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        old(info);
    }));
    run_ui(&args, &mut config, &db, capture, &interface, &stop)
}
fn main() {
    if let Err(e) = run() {
        eprintln!("global-network-map: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_origin_is_rejected() {
        for text in [
            "City,Country,NaN,0",
            "City,Country,91,0",
            "City,Country,0,181",
            ",Country,0,0",
            "City,Country",
        ] {
            assert!(parse_origin(text).is_err());
        }
        assert!(parse_origin("Phoenix,US,33.4484,-112.074").is_ok());
    }
}

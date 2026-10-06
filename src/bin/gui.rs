#[path = "../gui/mod.rs"]
mod gui;

use clap::Parser;
use eframe::egui;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "A graphical world map of this computer's network traffic"
)]
pub struct Args {
    /// Show clearly labeled synthetic traffic without capture permissions
    #[arg(long)]
    pub demo: bool,
    /// Open fullscreen; F11 toggles fullscreen in the window
    #[arg(long)]
    pub fullscreen: bool,
    /// Select a capture interface instead of automatic routing selection
    #[arg(short, long)]
    pub interface: Option<String>,
    /// Request capture after opening the window (uses desktop authentication)
    #[arg(long, conflicts_with = "demo")]
    pub start: bool,
    /// Use a local city MMDB instead of the default downloaded databases
    #[arg(long)]
    pub database: Option<PathBuf>,
    /// Save this window's rendered image as a PNG and exit (demo mode only)
    #[arg(long, requires = "demo")]
    pub screenshot: Option<PathBuf>,
    /// Exit after N seconds, useful for launch checks
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..=3600))]
    pub quit_after: Option<u64>,
}
fn main() -> anyhow::Result<()> {
    if unsafe { libc::geteuid() } == 0 {
        anyhow::bail!(
            "Run the graphical app as your normal user; it authorizes only the capture helper."
        );
    }
    let args = Args::parse();
    let fullscreen = args.fullscreen || args.screenshot.is_some();
    let mut app = gui::NetworkMapApp::new(args)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Global Network Map")
            .with_app_id("global-network-map")
            .with_inner_size([1440., 960.])
            .with_min_inner_size([960., 720.])
            .with_fullscreen(fullscreen),
        renderer: eframe::Renderer::Glow,
        multisampling: 4,
        ..Default::default()
    };
    eframe::run_native(
        "Global Network Map",
        options,
        Box::new(move |cc| {
            app.palette.apply(&cc.egui_ctx);
            app.setup_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Cannot open graphical window: {e}"))
}

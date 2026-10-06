use global_network_map::config::theme_paths;
use ratatui::style::Color;
use std::fs;

#[derive(Clone)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub accent: Color,
    pub muted: Color,
    pub border: Color,
    pub incoming: Color,
    pub outgoing: Color,
    pub panel: Color,
    pub source: String,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::Reset,
            foreground: Color::Reset,
            accent: Color::Cyan,
            muted: Color::DarkGray,
            border: Color::DarkGray,
            incoming: Color::Red,
            outgoing: Color::Green,
            panel: Color::Reset,
            source: "Terminal palette".into(),
        }
    }
}
fn hex(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
impl Theme {
    pub fn load() -> Self {
        for path in theme_paths() {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = text.parse::<toml::Value>() else {
                continue;
            };
            let get = |names: &[&str], fallback: Color| {
                names
                    .iter()
                    .find_map(|name| value.get(name).and_then(toml::Value::as_str).and_then(hex))
                    .unwrap_or(fallback)
            };
            let mut theme = Self::default();
            theme.background = get(&["background"], theme.background);
            theme.foreground = get(&["foreground"], theme.foreground);
            theme.accent = get(&["accent", "blue", "color4"], theme.accent);
            theme.muted = get(&["muted", "dark_foreground", "color8"], theme.muted);
            theme.border = get(
                &["selection", "selection_background", "color8"],
                theme.border,
            );
            theme.incoming = get(&["red", "color1"], theme.incoming);
            theme.outgoing = get(&["green", "color2"], theme.outgoing);
            theme.panel = get(&["lighter_background", "background"], theme.background);
            theme.source = "Omarchy · live palette".into();
            return theme;
        }
        Self::default()
    }
}

use eframe::egui::{self, Color32, Stroke};
use global_network_map::config::theme_paths;

#[derive(Clone, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub incoming: Color32,
    pub outgoing: Color32,
    pub border: Color32,
    pub source: String,
}
pub fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    let blend = |x: u8, y: u8| (x as f32 * (1. - amount) + y as f32 * amount).round() as u8;
    Color32::from_rgb(
        blend(a.r(), b.r()),
        blend(a.g(), b.g()),
        blend(a.b(), b.b()),
    )
}
fn hex(text: &str) -> Option<Color32> {
    let s = text.strip_prefix('#').unwrap_or(text);
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
impl Default for Palette {
    fn default() -> Self {
        Self {
            bg: Color32::from_rgb(7, 20, 38),
            panel: Color32::from_rgb(16, 39, 68),
            text: Color32::from_rgb(214, 230, 245),
            muted: Color32::from_rgb(114, 137, 166),
            accent: Color32::from_rgb(108, 182, 255),
            incoming: Color32::from_rgb(240, 127, 145),
            outgoing: Color32::from_rgb(133, 207, 187),
            border: Color32::from_rgb(32, 60, 96),
            source: "Built-in palette".into(),
        }
    }
}
impl Palette {
    pub fn load() -> Self {
        for path in theme_paths() {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(value) = text.parse::<toml::Value>() {
                    return Self::from_toml(&value);
                }
            }
        }
        Self::default()
    }
    fn from_toml(value: &toml::Value) -> Self {
        let mut p = Self::default();
        let get = |names: &[&str], fallback| {
            names
                .iter()
                .find_map(|n| value.get(n).and_then(toml::Value::as_str).and_then(hex))
                .unwrap_or(fallback)
        };
        p.bg = get(&["background"], p.bg);
        p.text = get(&["foreground"], p.text);
        p.accent = get(&["accent", "blue", "color4"], p.accent);
        p.muted = get(
            &["muted", "dark_foreground", "color8"],
            mix(p.bg, p.text, 0.55),
        );
        p.panel = get(&["lighter_background"], mix(p.bg, p.text, 0.035));
        p.border = get(
            &["selection", "selection_background", "color8"],
            mix(p.bg, p.text, 0.15),
        );
        p.incoming = get(&["red", "color1"], p.incoming);
        p.outgoing = get(&["green", "color2"], p.outgoing);
        p.source = "Omarchy · live theme".into();
        p
    }
    pub fn apply(&self, ctx: &egui::Context) {
        let light = self.bg.r() as u16 + self.bg.g() as u16 + self.bg.b() as u16 > 420;
        let mut v = if light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        v.override_text_color = Some(self.text);
        v.panel_fill = self.bg;
        v.window_fill = self.panel;
        v.extreme_bg_color = self.bg;
        v.faint_bg_color = mix(self.bg, self.panel, 0.5);
        v.window_stroke = Stroke::new(1.0_f32, self.border);
        v.selection.bg_fill = mix(self.bg, self.accent, 0.25);
        v.selection.stroke = Stroke::new(1.0_f32, self.accent);
        v.widgets.inactive.bg_fill = self.panel;
        v.widgets.inactive.weak_bg_fill = self.panel;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, self.border);
        v.widgets.hovered.bg_fill = mix(self.panel, self.accent, 0.16);
        v.widgets.hovered.weak_bg_fill = v.widgets.hovered.bg_fill;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, self.accent);
        v.widgets.active.bg_fill = mix(self.panel, self.accent, 0.25);
        v.hyperlink_color = self.accent;
        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(10., 8.);
            s.spacing.button_padding = egui::vec2(14., 8.);
            s.text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(18.));
            s.text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(18.));
            s.text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(18.));
            s.text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(18.));
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supports_named_and_numbered_omarchy_palettes() {
        for text in [
            "red='#010203'\ngreen='#040506'",
            "color1='#010203'\ncolor2='#040506'",
        ] {
            let p = Palette::from_toml(&text.parse().unwrap());
            assert_eq!(p.incoming, Color32::from_rgb(1, 2, 3));
            assert_eq!(p.outgoing, Color32::from_rgb(4, 5, 6));
        }
    }
}

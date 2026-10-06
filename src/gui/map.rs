use super::palette::{mix, Palette};
use anyhow::{Context, Result};
use eframe::egui::{
    self, vec2, Align2, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use global_network_map::{
    config::Origin,
    model::{bytes, Model},
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc, time::Instant};

#[derive(Deserialize)]
struct Country {
    name: String,
    polygons: Vec<Vec<Vec<[f64; 2]>>>,
}
struct Land {
    vertices: Vec<[f32; 2]>,
    indices: Vec<u32>,
    rings: Vec<Vec<[f32; 2]>>,
}
#[derive(Clone, PartialEq)]
struct CacheKey {
    rect: Rect,
    zoom: f32,
    pan: Vec2,
    palette: Palette,
}
pub struct WorldMap {
    land: Vec<Land>,
    pub countries: usize,
    pub vertices: usize,
    pub zoom: f32,
    pan: Vec2,
    cache: Option<(CacheKey, Vec<Shape>)>,
}
#[derive(Clone, Copy)]
struct Projection {
    rect: Rect,
    zoom: f32,
    pan: Vec2,
}
impl Projection {
    fn scale(self) -> Vec2 {
        vec2(self.rect.width() / 360., self.rect.height() / 180.) * self.zoom
    }
    fn point(self, lon: f32, lat: f32) -> Pos2 {
        self.rect.center() + vec2(lon + self.pan.x, -lat + self.pan.y) * self.scale()
    }
    fn inverse(self, p: Pos2) -> [f32; 2] {
        let q = (p - self.rect.center()) / self.scale() - self.pan;
        [q.x, -q.y]
    }
}
#[derive(Clone)]
pub struct CityNode {
    pub city: String,
    pub country: String,
    pub longitude: f32,
    pub latitude: f32,
    pub rates: [f64; 2],
    pub total: [u64; 2],
    pub count: usize,
    pub last: [Option<Instant>; 2],
}
pub fn city_nodes(model: &Model) -> Vec<CityNode> {
    let mut groups: HashMap<String, CityNode> = HashMap::new();
    for (_, endpoint) in model.active() {
        let Some((lon, lat)) = endpoint.location.coordinates() else {
            continue;
        };
        let key = format!(
            "{}:{}:{lon:.2}:{lat:.2}",
            endpoint.location.city, endpoint.location.country
        );
        let n = groups.entry(key).or_insert_with(|| CityNode {
            city: endpoint.location.city.clone(),
            country: endpoint.location.country.clone(),
            longitude: lon as f32,
            latitude: lat as f32,
            rates: [0.; 2],
            total: [0; 2],
            count: 0,
            last: [None, None],
        });
        n.count += 1;
        for i in 0..2 {
            n.rates[i] += endpoint.counter.rate[i];
            n.total[i] += endpoint.counter.total[i];
            n.last[i] = n.last[i].max(endpoint.last[i]);
        }
    }
    let mut nodes: Vec<_> = groups.into_values().collect();
    nodes.sort_by(|a, b| {
        b.rates
            .iter()
            .sum::<f64>()
            .total_cmp(&a.rates.iter().sum::<f64>())
            .then_with(|| a.city.cmp(&b.city))
    });
    nodes
}
pub fn arc(origin: [f32; 2], end: [f32; 2], t: f32, direction: usize) -> [f32; 2] {
    let delta = (end[0] - origin[0] + 540.).rem_euclid(360.) - 180.;
    let bend = (delta.abs() * 0.14 + 5.).min(27.) + if direction == 0 { 1.0 } else { -1.0 };
    [
        (origin[0] + delta * t + 180.).rem_euclid(360.) - 180.,
        (origin[1] + (end[1] - origin[1]) * t + 4. * t * (1. - t) * bend).clamp(-89.5, 89.5),
    ]
}
impl WorldMap {
    pub fn load() -> Result<Self> {
        let countries: Vec<Country> =
            serde_json::from_str(include_str!("../../assets/world-50m.json"))?;
        let count = countries.len();
        let mut land = vec![];
        let mut vertex_count = 0;
        for country in countries {
            for polygon in country.polygons {
                let mut flat = vec![];
                let mut holes = vec![];
                let mut rings = vec![];
                for (i, mut ring) in polygon.into_iter().enumerate() {
                    if ring.first() == ring.last() {
                        ring.pop();
                    }
                    if ring.len() < 3 {
                        continue;
                    }
                    if i > 0 {
                        holes.push(flat.len() / 2);
                    }
                    for point in &ring {
                        flat.extend_from_slice(point);
                    }
                    rings.push(
                        ring.into_iter()
                            .map(|p| [p[0] as f32, p[1] as f32])
                            .collect(),
                    );
                }
                if flat.len() < 6 {
                    continue;
                }
                let indices = earcutr::earcut(&flat, &holes, 2)
                    .with_context(|| format!("Cannot triangulate {}", country.name))?
                    .into_iter()
                    .map(|i| i as u32)
                    .collect();
                let vertices: Vec<_> = flat
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| [p[0] as f32, p[1] as f32])
                    .collect();
                vertex_count += vertices.len();
                land.push(Land {
                    vertices,
                    indices,
                    rings,
                });
            }
        }
        Ok(Self {
            land,
            countries: count,
            vertices: vertex_count,
            zoom: 1.,
            pan: Vec2::ZERO,
            cache: None,
        })
    }
    pub fn reset(&mut self) {
        self.zoom = 1.;
        self.pan = Vec2::ZERO;
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        p: &Palette,
        nodes: &[CityNode],
        origin: Option<&Origin>,
        phase: f32,
    ) {
        let response = ui.interact(rect, ui.id().with("world-map"), Sense::click_and_drag());
        let map_rect = Rect::from_min_max(rect.min + vec2(12., 48.), rect.max - vec2(12., 88.));
        let mut projection = Projection {
            rect: map_rect,
            zoom: self.zoom,
            pan: self.pan,
        };
        if response.dragged() {
            self.pan += response.drag_delta() / projection.scale();
        }
        if response.double_clicked() {
            self.reset();
        }
        if let Some(pointer) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.01 {
                let anchor = projection.inverse(pointer);
                self.zoom = (self.zoom * (scroll * 0.003).exp()).clamp(1., 8.);
                projection.zoom = self.zoom;
                let q = (pointer - map_rect.center()) / projection.scale();
                self.pan = vec2(q.x - anchor[0], q.y + anchor[1]);
            }
        }
        self.pan.x = self.pan.x.clamp(-360., 360.);
        self.pan.y = self.pan.y.clamp(-180., 180.);
        projection.zoom = self.zoom;
        projection.pan = self.pan;
        let key = CacheKey {
            rect: map_rect,
            zoom: self.zoom,
            pan: self.pan,
            palette: p.clone(),
        };
        if self.cache.as_ref().is_none_or(|(k, _)| *k != key) {
            self.cache = Some((key, self.base_shapes(projection, p)));
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10., mix(p.bg, p.panel, 0.2));
        let canvas = painter.with_clip_rect(map_rect);
        if let Some((_, shapes)) = &self.cache {
            canvas.extend(shapes.iter().cloned());
        }
        if let Some(origin) = origin {
            let home = [origin.longitude as f32, origin.latitude as f32];
            for (index, node) in nodes.iter().take(256).enumerate() {
                for direction in 0..2 {
                    let Some(last) = node.last[direction] else {
                        continue;
                    };
                    let age = last.elapsed().as_secs_f32();
                    if age > 8. {
                        continue;
                    }
                    let color = if direction == 0 {
                        p.incoming
                    } else {
                        p.outgoing
                    };
                    let end = [node.longitude, node.latitude];
                    let mut segment = vec![];
                    let mut last_lon = home[0];
                    for i in 0..=80 {
                        let point = arc(home, end, i as f32 / 80., direction);
                        if (point[0] - last_lon).abs() > 180. && segment.len() > 1 {
                            canvas.add(Shape::line(
                                std::mem::take(&mut segment),
                                Stroke::new(1.0_f32, color.gamma_multiply(0.35)),
                            ));
                        }
                        segment.push(projection.point(point[0], point[1]));
                        last_lon = point[0];
                    }
                    if segment.len() > 1 {
                        canvas.add(Shape::line(
                            segment.clone(),
                            Stroke::new(4.0_f32, color.gamma_multiply(0.045)),
                        ));
                        canvas.add(Shape::line(
                            segment,
                            Stroke::new(
                                1.0_f32,
                                color.gamma_multiply(if age < 2. { 0.5 } else { 0.2 }),
                            ),
                        ));
                    }
                    if age < 3. {
                        for trail in 0..8 {
                            let t = (phase * 0.32 + index as f32 * 0.127 - trail as f32 * 0.012)
                                .rem_euclid(1.);
                            let point = arc(
                                home,
                                end,
                                if direction == 0 { 1. - t } else { t },
                                direction,
                            );
                            let position = projection.point(point[0], point[1]);
                            let alpha = 1. - trail as f32 / 9.;
                            canvas.circle_filled(
                                position,
                                5. - trail as f32 * 0.3,
                                color.gamma_multiply(alpha * 0.07),
                            );
                            canvas.circle_filled(
                                position,
                                2. - trail as f32 * 0.12,
                                color.gamma_multiply(alpha),
                            );
                        }
                    }
                }
            }
        }
        let mut occupied: Vec<Rect> = vec![];
        if let Some(origin) = origin {
            let position = projection.point(origin.longitude as f32, origin.latitude as f32);
            if map_rect.contains(position) {
                canvas.circle_filled(position, 12., p.accent.gamma_multiply(0.07));
                canvas.circle_stroke(position, 7., Stroke::new(1.3_f32, p.accent));
                canvas.circle_filled(position, 3., p.text);
                let label = canvas.text(
                    position + vec2(11., -11.),
                    Align2::LEFT_BOTTOM,
                    &origin.city,
                    FontId::proportional(18.),
                    p.text,
                );
                occupied.push(label.expand(4.));
                let hover = ui.interact(
                    Rect::from_center_size(position, vec2(18., 18.)),
                    ui.id().with("origin"),
                    Sense::hover(),
                );
                hover.on_hover_ui(|ui| {
                    ui.strong(format!("{}, {}", origin.city, origin.country));
                    ui.label("Your configured map origin");
                });
            }
        }
        for (index, node) in nodes.iter().take(256).enumerate() {
            let position = projection.point(node.longitude, node.latitude);
            if !map_rect.contains(position) {
                continue;
            }
            let color = if node.last[0] >= node.last[1] {
                p.incoming
            } else {
                p.outgoing
            };
            let pulse = 1. + ((phase * 2. + index as f32).sin() + 1.) * 0.3;
            canvas.circle_filled(position, 8. * pulse, color.gamma_multiply(0.08));
            canvas.circle_filled(position, 4., color.gamma_multiply(0.4));
            canvas.circle_filled(position, 2.4, color);
            let label_position = position + vec2(8., -6.);
            let galley =
                canvas.layout_no_wrap(node.city.clone(), FontId::proportional(18.), p.text);
            let label_rect = Rect::from_min_size(label_position, galley.size());
            if index < 18
                && map_rect.contains_rect(label_rect)
                && !occupied.iter().any(|r| r.intersects(label_rect))
            {
                canvas.rect_filled(
                    label_rect.expand2(vec2(3., 1.)),
                    2.,
                    p.bg.gamma_multiply(0.88),
                );
                canvas.galley(label_position, galley, p.text);
                occupied.push(label_rect.expand(4.));
            }
            let hover_rect = Rect::from_center_size(position, vec2(16., 16.));
            let hover = ui.interact(hover_rect, ui.id().with(("city", index)), Sense::hover());
            hover.on_hover_ui_at_pointer(|ui| {
                ui.strong(format!("{}, {}", node.city, node.country));
                ui.label(format!(
                    "{} active endpoint{}",
                    node.count,
                    if node.count == 1 { "" } else { "s" }
                ));
                ui.separator();
                ui.colored_label(
                    p.incoming,
                    format!(
                        "↓ {}/s   ·   {} received",
                        bytes(node.rates[0]),
                        bytes(node.total[0] as f64)
                    ),
                );
                ui.colored_label(
                    p.outgoing,
                    format!(
                        "↑ {}/s   ·   {} sent",
                        bytes(node.rates[1]),
                        bytes(node.total[1] as f64)
                    ),
                );
                ui.small("Approximate IP location · displayed endpoints");
            });
        }
        painter.rect_stroke(
            rect,
            10.,
            Stroke::new(1.0_f32, p.border),
            StrokeKind::Inside,
        );
        painter.text(
            rect.left_top() + vec2(18., 14.),
            Align2::LEFT_TOP,
            "WORLD CONNECTIONS",
            FontId::monospace(18.),
            p.accent,
        );
        painter.text(
            rect.right_top() + vec2(-18., 14.),
            Align2::RIGHT_TOP,
            format!("{} CITIES   ·   {:.1}×", nodes.len(), self.zoom),
            FontId::monospace(18.),
            p.muted,
        );
        painter.text(
            rect.left_bottom() + vec2(18., -13.),
            Align2::LEFT_BOTTOM,
            "Scroll: zoom · Drag: pan",
            FontId::proportional(18.),
            p.muted,
        );
        painter.text(
            rect.right_bottom() + vec2(-18., -13.),
            Align2::RIGHT_BOTTOM,
            "Natural Earth",
            FontId::monospace(18.),
            p.muted,
        );
    }
    fn base_shapes(&self, projection: Projection, p: &Palette) -> Vec<Shape> {
        let mut shapes = vec![];
        let grid = mix(p.bg, p.border, 0.38);
        for lon in (-180..=180).step_by(30) {
            shapes.push(Shape::line_segment(
                [
                    projection.point(lon as f32, -90.),
                    projection.point(lon as f32, 90.),
                ],
                Stroke::new(0.5_f32, grid),
            ));
        }
        for lat in (-90..=90).step_by(30) {
            shapes.push(Shape::line_segment(
                [
                    projection.point(-180., lat as f32),
                    projection.point(180., lat as f32),
                ],
                Stroke::new(0.5_f32, grid),
            ));
        }
        // Cache screen-space meshes; steady animation only redraws small tracer paths.
        let mut mesh = Mesh::default();
        for land in &self.land {
            let offset = mesh.vertices.len() as u32;
            for &[lon, lat] in &land.vertices {
                let tint = 0.14 + 0.035 * (lat.to_radians().sin() + 1.);
                mesh.colored_vertex(projection.point(lon, lat), mix(p.bg, p.accent, tint));
            }
            mesh.indices.extend(land.indices.iter().map(|i| i + offset));
        }
        shapes.push(Shape::Mesh(Arc::new(mesh)));
        for land in &self.land {
            for ring in &land.rings {
                let points = ring.iter().map(|p| projection.point(p[0], p[1])).collect();
                shapes.push(Shape::closed_line(
                    points,
                    Stroke::new(0.65_f32, mix(p.bg, p.accent, 0.4)),
                ));
            }
        }
        shapes
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;
    #[test]
    fn world_polygons_triangulate_with_holes_and_valid_indices() {
        let world = WorldMap::load().unwrap();
        assert!(world.countries > 200);
        assert!(world.vertices > 50000);
        for land in world.land {
            assert!(land.indices.len() % 3 == 0);
            assert!(land
                .indices
                .iter()
                .all(|i| (*i as usize) < land.vertices.len()));
        }
    }
    #[test]
    fn projection_roundtrips_and_dateline_paths_stay_short() {
        let projection = Projection {
            rect: Rect::from_min_size(pos2(0., 0.), vec2(1000., 500.)),
            zoom: 2.,
            pan: vec2(10., 20.),
        };
        let point = projection.inverse(projection.point(-112., 33.));
        assert!((point[0] + 112.).abs() < 0.001);
        assert!((point[1] - 33.).abs() < 0.001);
        assert!(arc([170., 30.], [-170., 40.], 0.5, 0)[0].abs() > 170.);
    }
    #[test]
    fn city_hover_shows_country_and_both_transfer_directions() {
        let mut world = WorldMap::load().unwrap();
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.interaction.tooltip_delay = 0.);
        let palette = Palette::default();
        let node = CityNode {
            city: "Test City".into(),
            country: "Test Country".into(),
            longitude: 0.,
            latitude: 0.,
            rates: [1024., 2048.],
            total: [4096, 8192],
            count: 2,
            last: [Some(Instant::now()); 2],
        };
        let rect = Rect::from_min_size(pos2(0., 0.), vec2(1000., 700.));
        let mut texts = Vec::new();
        fn collect(shape: &Shape, texts: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texts)),
                _ => {}
            }
        }
        // Egui needs previous-frame hitboxes before the pointer can open a tooltip.
        for frame in 0..4 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    time: Some(frame as f64),
                    events: vec![egui::Event::PointerMoved(rect.center() - vec2(0., 20.))],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        world.show(ui, rect, &palette, std::slice::from_ref(&node), None, 0.);
                    });
                },
            );
            for shape in output.shapes {
                collect(&shape.shape, &mut texts);
            }
        }
        assert!(
            texts.iter().any(|s| s == "Test City, Test Country"),
            "{texts:?}"
        );
        assert!(texts.iter().any(|s| s.contains("received")));
        assert!(texts.iter().any(|s| s.contains("sent")));
    }
}

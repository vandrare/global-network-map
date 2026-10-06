use super::palette::{mix, Palette};
use eframe::egui::{self, pos2, Align2, FontId, Rect, Sense, Shape, Stroke, StrokeKind};
use global_network_map::model::bytes;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub time: f64,
    pub rates: [f64; 2],
}
#[derive(Default)]
pub struct TrafficChart {
    pub samples: VecDeque<Sample>,
}
impl TrafficChart {
    pub fn push(&mut self, time: f64, rates: [f64; 2]) {
        self.samples.push_back(Sample { time, rates });
        // Keep one extra sample for the segment crossing the left edge.
        while self.samples.len() > 2 && self.samples[1].time < time - 60. {
            self.samples.pop_front();
        }
    }
    pub fn nearest(&self, time: f64) -> Option<Sample> {
        self.samples
            .iter()
            .min_by(|a, b| (a.time - time).abs().total_cmp(&(b.time - time).abs()))
            .copied()
    }
    pub fn show(&self, ui: &mut egui::Ui, rect: Rect, p: &Palette, now: f64) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10., mix(p.bg, p.panel, 0.38));
        painter.rect_stroke(
            rect,
            10.,
            Stroke::new(1.0_f32, p.border),
            StrokeKind::Inside,
        );
        painter.text(
            rect.left_top() + egui::vec2(18., 17.),
            Align2::LEFT_TOP,
            "TRANSFER HISTORY",
            FontId::monospace(18.),
            p.accent,
        );
        painter.text(
            rect.right_top() + egui::vec2(-18., 17.),
            Align2::RIGHT_TOP,
            "LAST 60 SECONDS",
            FontId::monospace(18.),
            p.muted,
        );
        let plot = Rect::from_min_max(
            rect.left_top() + egui::vec2(118., 57.),
            rect.right_bottom() - egui::vec2(22., 72.),
        );
        let peak = self
            .samples
            .iter()
            .filter(|s| s.time >= now - 60.)
            .flat_map(|s| s.rates)
            .fold(1., f64::max);
        let max = axis_max(peak);
        for i in 0..=4 {
            let y = egui::lerp(plot.bottom()..=plot.top(), i as f32 / 4.);
            painter.line_segment(
                [pos2(plot.left(), y), pos2(plot.right(), y)],
                Stroke::new(1.0_f32, mix(p.bg, p.border, 0.6)),
            );
            painter.text(
                pos2(plot.left() - 10., y),
                Align2::RIGHT_CENTER,
                bytes(max * i as f64 / 4.),
                FontId::monospace(18.),
                p.muted,
            );
        }
        for (fraction, label) in [(0., "60s ago"), (0.5, "30s"), (1., "Now")] {
            let x = egui::lerp(plot.left()..=plot.right(), fraction);
            painter.text(
                pos2(x, plot.bottom() + 12.),
                if fraction == 0. {
                    Align2::LEFT_TOP
                } else if fraction == 1. {
                    Align2::RIGHT_TOP
                } else {
                    Align2::CENTER_TOP
                },
                label,
                FontId::monospace(18.),
                p.muted,
            );
        }
        let x = |time: f64| plot.right() - (now - time) as f32 / 60. * plot.width();
        let y = |rate: f64| plot.bottom() - (rate / max) as f32 * plot.height();
        let clipped = painter.with_clip_rect(plot);
        for (direction, color) in [p.incoming, p.outgoing].into_iter().enumerate() {
            let points: Vec<_> = self
                .samples
                .iter()
                .map(|s| pos2(x(s.time), y(s.rates[direction])))
                .collect();
            if points.len() > 1 {
                clipped.add(Shape::line(
                    points.clone(),
                    Stroke::new(7.0_f32, color.gamma_multiply(0.1)),
                ));
                clipped.add(Shape::line(points.clone(), Stroke::new(2.0_f32, color)));
                if let Some(last) = points.last() {
                    clipped.circle_filled(*last, 3.5, color);
                }
            }
        }
        if self.samples.len() < 2 {
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                "Collecting transfer samples…",
                FontId::proportional(18.),
                p.muted,
            );
        }
        // The graph uses observed samples and linear interpolation, never a spline
        // that could invent negative rates or overshoot a real throughput peak.
        let response = ui.interact(plot, ui.id().with("traffic-chart"), Sense::hover());
        if let Some(pointer) = response.hover_pos() {
            let time = now - (plot.right() - pointer.x) as f64 / plot.width() as f64 * 60.;
            if let Some(sample) = self.nearest(time).filter(|s| (s.time - time).abs() < 2.) {
                let sx = x(sample.time);
                clipped.line_segment(
                    [pos2(sx, plot.top()), pos2(sx, plot.bottom())],
                    Stroke::new(1.0_f32, p.muted),
                );
                for (i, color) in [p.incoming, p.outgoing].into_iter().enumerate() {
                    clipped.circle_filled(pos2(sx, y(sample.rates[i])), 4., color);
                }
                response.on_hover_ui_at_pointer(|ui| {
                    ui.label(format!("{:.1} seconds ago", (now - sample.time).max(0.)));
                    ui.colored_label(
                        p.incoming,
                        format!("↓ Download  {}/s", bytes(sample.rates[0])),
                    );
                    ui.colored_label(
                        p.outgoing,
                        format!("↑ Upload      {}/s", bytes(sample.rates[1])),
                    );
                });
            }
        }
        let legend_y = rect.bottom() - 16.;
        painter.text(
            pos2(rect.left() + 18., legend_y),
            Align2::LEFT_CENTER,
            "↓ Download",
            FontId::proportional(18.),
            p.incoming,
        );
        painter.text(
            pos2(rect.left() + 180., legend_y),
            Align2::LEFT_CENTER,
            "↑ Upload",
            FontId::proportional(18.),
            p.outgoing,
        );
        painter.text(
            pos2(rect.right() - 18., legend_y),
            Align2::RIGHT_CENTER,
            "bytes / second",
            FontId::monospace(18.),
            p.muted,
        );
    }
}
fn axis_max(peak: f64) -> f64 {
    let magnitude = 10_f64.powf(peak.max(1.).log10().floor());
    let scaled = peak / magnitude;
    [1., 2., 5., 10.]
        .into_iter()
        .find(|n| *n >= scaled)
        .unwrap_or(10.)
        * magnitude
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_time_based_and_hover_finds_actual_sample() {
        let mut chart = TrafficChart::default();
        for i in 0..100 {
            chart.push(i as f64 * 1.3, [i as f64, 2. * i as f64]);
        }
        let last = chart.samples.back().unwrap().time;
        assert!(chart.samples[1].time >= last - 60.);
        assert_eq!(chart.nearest(last - 1.4).unwrap().rates, [98., 196.]);
        assert!(axis_max(123456.) >= 123456.);
    }
}

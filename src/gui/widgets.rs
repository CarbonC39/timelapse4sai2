use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Refresh,
    Folder,
    Play,
    Stop,
    Export,
    Check,
    Warning,
    ZoomIn,
    ZoomOut,
    Fit,
    Move,
}

pub fn paint_icon(ui: &egui::Ui, icon: Icon, center: Pos2, size: f32, color: Color32) {
    let painter = ui.painter();
    let at = |x: f32, y: f32| center + Vec2::new(x, y) * size;
    let stroke = Stroke::new((size / 10.0).max(1.4), color);
    let line = |a, b| {
        painter.line_segment([a, b], stroke);
    };
    match icon {
        Icon::Refresh => {
            let points = (0..=24)
                .map(|i| {
                    let angle = 0.25 + i as f32 / 24.0 * 5.0;
                    at(angle.cos() * 0.36, angle.sin() * 0.36)
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            line(at(0.18, -0.41), at(0.39, -0.32));
            line(at(0.39, -0.32), at(0.34, -0.10));
        }
        Icon::Folder => {
            painter.add(egui::Shape::closed_line(
                vec![
                    at(-0.43, 0.32),
                    at(-0.43, -0.30),
                    at(-0.10, -0.30),
                    at(0.02, -0.15),
                    at(0.43, -0.15),
                    at(0.43, 0.32),
                ],
                stroke,
            ));
        }
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![at(-0.24, -0.36), at(0.36, 0.0), at(-0.24, 0.36)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            painter.rect_filled(
                egui::Rect::from_center_size(center, Vec2::splat(size * 0.60)),
                2.0,
                color,
            );
        }
        Icon::Export => {
            line(at(-0.38, 0.12), at(-0.38, 0.38));
            line(at(-0.38, 0.38), at(0.38, 0.38));
            line(at(0.38, 0.38), at(0.38, 0.12));
            line(at(0.0, 0.12), at(0.0, -0.40));
            line(at(0.0, -0.40), at(-0.22, -0.17));
            line(at(0.0, -0.40), at(0.22, -0.17));
        }
        Icon::Check => {
            line(at(-0.36, 0.0), at(-0.08, 0.28));
            line(at(-0.08, 0.28), at(0.38, -0.28));
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            painter.circle_stroke(at(-0.06, -0.06), size * 0.27, stroke);
            line(at(0.15, 0.15), at(0.40, 0.40));
            line(at(-0.20, -0.06), at(0.08, -0.06));
            if matches!(icon, Icon::ZoomIn) {
                line(at(-0.06, -0.20), at(-0.06, 0.08));
            }
        }
        Icon::Fit => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                line(at(x * 0.18, y * 0.40), at(x * 0.40, y * 0.40));
                line(at(x * 0.40, y * 0.40), at(x * 0.40, y * 0.18));
            }
        }
        Icon::Move => {
            line(at(-0.4, 0.0), at(0.4, 0.0));
            line(at(0.0, -0.4), at(0.0, 0.4));
            for (x, y) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                line(
                    at(x * 0.4, y * 0.4),
                    at(x * 0.22 - y * 0.12, y * 0.22 + x * 0.12),
                );
                line(
                    at(x * 0.4, y * 0.4),
                    at(x * 0.22 + y * 0.12, y * 0.22 - x * 0.12),
                );
            }
        }
        Icon::Warning => {
            painter.add(egui::Shape::closed_line(
                vec![at(0.0, -0.43), at(0.44, 0.35), at(-0.44, 0.35)],
                stroke,
            ));
            line(at(0.0, -0.15), at(0.0, 0.08));
            painter.circle_filled(at(0.0, 0.23), size * 0.045, color);
        }
    }
}

pub fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool) -> Response {
    let size = ui.spacing().interact_size.y.max(28.0);
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
            let visuals = ui.style().interact(&response);
            ui.painter().rect(
                rect,
                5.0,
                visuals.bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
            paint_icon(
                ui,
                icon,
                rect.center(),
                size * 0.58,
                visuals.fg_stroke.color,
            );
            response
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response.on_hover_text(label)
}

pub fn action_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    stop: bool,
) -> Response {
    let height = (ui.text_style_height(&egui::TextStyle::Button) + 20.0).max(38.0);
    let color = if stop {
        Color32::from_rgb(177, 48, 54)
    } else {
        Color32::from_rgb(38, 105, 180)
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .fill(if enabled {
                color
            } else {
                ui.visuals().widgets.inactive.bg_fill
            })
            .min_size(egui::vec2(ui.available_width(), height)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let text_color = if enabled {
        Color32::WHITE
    } else {
        ui.visuals().weak_text_color()
    };
    let painter = ui.painter().with_clip_rect(response.rect);
    if enabled && response.hovered() {
        painter.rect_filled(response.rect, 4.0, color.gamma_multiply(1.12));
    }
    paint_icon(
        ui,
        icon,
        egui::pos2(response.rect.left() + 22.0, response.rect.center().y),
        17.0,
        text_color,
    );
    painter.text(
        egui::pos2(response.rect.left() + 44.0, response.rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        text_color,
    );
    response
}

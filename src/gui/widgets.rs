use super::i18n::{Key, Translator};
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
}

#[derive(Clone, Copy)]
pub enum ActionStyle {
    Primary,
    Secondary,
    Stop,
}

pub fn form_label(ui: &mut egui::Ui, i18n: Translator, key: Key) -> Response {
    let color = ui.visuals().text_color();
    let width = (ui.available_width() * 0.34).clamp(72.0, 184.0);
    let galley = ui.painter().layout(
        i18n.tr(key).to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        color,
        width,
    );
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, galley.size().y.max(28.0)), Sense::hover());
    ui.painter().galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), i18n.tr(key))
    });
    response
}

fn help_size(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y.min(22.0).max(18.0)
}

/// Reserve the same width as a help icon for rows without a tooltip, keeping
/// control columns aligned throughout an advanced panel.
pub fn help_slot(ui: &mut egui::Ui) {
    ui.allocate_exact_size(Vec2::splat(help_size(ui)), Sense::hover());
}

/// Small, quiet `?` affordance placed after a field label. The tooltip is the
/// explanation body only; the label is already visible next to the icon.
pub fn help_icon(ui: &mut egui::Ui, i18n: Translator, tip: Key) -> Response {
    let size = help_size(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let color = if response.hovered() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    if response.hovered() {
        ui.painter()
            .circle_stroke(rect.center(), size * 0.46, Stroke::new(1.0, color));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "?",
        egui::TextStyle::Small.resolve(ui.style()),
        color,
    );
    let text = i18n.tr(tip);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text)
    });
    response.on_hover_text(text)
}

pub fn metrics_row(
    ui: &mut egui::Ui,
    i18n: Translator,
    values: &[(Key, String)],
    base_size: f32,
) {
    if values.is_empty() {
        return;
    }
    ui.columns(values.len(), |columns| {
        for (column, (key, value)) in columns.iter_mut().zip(values) {
            column.label(
                egui::RichText::new(i18n.tr(*key))
                    .size(base_size * 12.0 / 14.5)
                    .color(column.visuals().weak_text_color()),
            );
            column.label(egui::RichText::new(value).size(base_size * 15.0 / 14.5));
        }
    });
}

pub fn advanced_toggle(ui: &mut egui::Ui, i18n: Translator, open: &mut bool) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    let galley = ui
        .painter()
        .layout_no_wrap(i18n.tr(Key::Advanced).to_owned(), font, color);
    let width = galley.size().x + 24.0;
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(Vec2::new(width, 28.0)),
    );
    if response.clicked() {
        *open = !*open;
    }
    let color = if response.hovered() {
        ui.visuals().text_color()
    } else {
        color
    };
    let center = egui::pos2(response.rect.left() + 8.0, response.rect.center().y);
    let offsets = if *open {
        [
            Vec2::new(-4.0, -2.0),
            Vec2::new(4.0, -2.0),
            Vec2::new(0.0, 3.0),
        ]
    } else {
        [
            Vec2::new(-2.0, -4.0),
            Vec2::new(-2.0, 4.0),
            Vec2::new(3.0, 0.0),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        offsets.into_iter().map(|p| center + p).collect(),
        color,
        Stroke::NONE,
    ));
    ui.painter().galley(
        egui::pos2(
            response.rect.left() + 20.0,
            response.rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        color,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            4.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            i18n.tr(Key::Advanced),
        )
    });
    response
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
    style: ActionStyle,
) -> Response {
    let height = (ui.text_style_height(&egui::TextStyle::Button) + 10.0).clamp(28.0, 30.0);
    let accent = match style {
        ActionStyle::Primary => Color32::from_rgb(38, 105, 180),
        ActionStyle::Stop => Color32::from_rgb(177, 48, 54),
        ActionStyle::Secondary => ui.visuals().widgets.inactive.bg_fill,
    };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let label_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    });
    let width = (label_width + 58.0)
        .max(130.0)
        .min(ui.available_width().max(0.0));
    let secondary = matches!(style, ActionStyle::Secondary);
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .fill(if secondary {
                ui.visuals().widgets.inactive.bg_fill
            } else if enabled {
                accent
            } else {
                ui.visuals().widgets.inactive.bg_fill
            })
            .stroke(if secondary {
                ui.visuals().widgets.inactive.bg_stroke
            } else {
                Stroke::NONE
            })
            .min_size(egui::vec2(width, height)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let text_color = if !enabled {
        ui.visuals().weak_text_color()
    } else if secondary {
        ui.visuals().text_color()
    } else {
        Color32::WHITE
    };
    let painter = ui.painter().with_clip_rect(response.rect);
    if enabled && response.hovered() && !secondary {
        painter.rect_filled(response.rect, 4.0, accent.gamma_multiply(1.12));
    }
    paint_icon(
        ui,
        icon,
        egui::pos2(response.rect.left() + 21.0, response.rect.center().y),
        16.0,
        text_color,
    );
    painter.text(
        egui::pos2(response.rect.left() + 39.0, response.rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        text_color,
    );
    if response.has_focus() {
        painter.rect_stroke(
            response.rect.expand(1.0),
            ui.visuals().widgets.active.corner_radius,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    response
}

pub fn path_edit(ui: &mut egui::Ui, value: &mut String, width: f32, hint: &str) -> Response {
    ui.scope(|ui| {
        let background = if ui.visuals().dark_mode {
            Color32::from_rgb(35, 38, 42)
        } else {
            Color32::from_rgb(235, 236, 238)
        };
        ui.visuals_mut().text_edit_bg_color = Some(background);
        let border = if ui.visuals().dark_mode {
            Color32::from_gray(65)
        } else {
            Color32::from_gray(190)
        };
        ui.visuals_mut().widgets.inactive.bg_stroke.color = border;
        ui.visuals_mut().widgets.hovered.bg_stroke.color = border;
        ui.add_sized(
            Vec2::new(width.max(0.0), ui.spacing().interact_size.y.max(28.0)),
            egui::TextEdit::singleline(value).hint_text(hint),
        )
    })
    .inner
}

use super::{
    i18n::{Key, tr},
    widgets::{Icon, paint_icon},
};
use eframe::egui::{self, Color32, Vec2};

pub struct Preview {
    texture: Option<egui::TextureHandle>,
    pub loading: bool,
    pub error: Option<String>,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            texture: None,
            loading: false,
            error: None,
        }
    }
}

impl Preview {
    pub fn clear(&mut self) {
        self.texture = None;
        self.error = None;
        self.loading = false;
    }

    pub fn set(
        &mut self,
        ctx: &egui::Context,
        result: Result<timelapse4sai2::preview::PreviewImage, String>,
    ) {
        self.loading = false;
        match result {
            Ok(image) => {
                self.texture = Some(ctx.load_texture(
                    "canvas-preview",
                    egui::ColorImage::from_rgb(
                        [image.width as usize, image.height as usize],
                        &image.rgb,
                    ),
                    egui::TextureOptions::LINEAR,
                ));
                self.error = None;
            }
            Err(error) => {
                self.texture = None;
                self.error = Some(error);
            }
        }
    }

    /// Draw the preview and its refresh control. Returns true when the user
    /// requests a manual refresh; preview data remains owned by the caller.
    pub fn show(&mut self, ui: &mut egui::Ui, recording: bool) -> bool {
        let available = ui.available_size_before_wrap();
        let footer_height = 34.0;
        let footer_gap = ui.spacing().item_spacing.y;
        let image_height = (available.y - footer_height - footer_gap).max(0.0);
        let (image_area, _) = ui.allocate_exact_size(
            Vec2::new(available.x.max(0.0), image_height),
            egui::Sense::hover(),
        );
        let canvas = Color32::from_rgb(42, 45, 49);
        ui.painter().rect_filled(image_area, 0.0, canvas);

        let padding = 32.0;
        let inner = image_area.shrink(padding);
        if let Some(texture) = &self.texture {
            let original = texture.size_vec2();
            if original.x > 0.0 && original.y > 0.0 && inner.width() > 0.0 && inner.height() > 0.0 {
                let scale = (inner.width() / original.x).min(inner.height() / original.y);
                let size = original * scale;
                let rect = egui::Rect::from_center_size(image_area.center(), size);
                let painter = ui.painter().with_clip_rect(image_area);
                painter.image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if recording {
                    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(155));
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        tr(Key::Recording),
                        egui::TextStyle::Heading.resolve(ui.style()),
                        Color32::WHITE,
                    );
                }
            }
        } else if self.loading {
            centered_message(ui, image_area, tr(Key::Loading));
        } else if let Some(error) = &self.error {
            centered_message(ui, image_area, tr(Key::PreviewUnavailable)).on_hover_text(error);
        } else {
            centered_message(ui, image_area, tr(Key::PreviewEmpty));
        }

        let (footer, _) = ui.allocate_exact_size(
            Vec2::new(available.x.max(0.0), footer_height),
            egui::Sense::hover(),
        );
        let label = tr(Key::Refresh);
        let tip = tr(Key::PreviewRefreshTip);
        let enabled = !self.loading;
        let text_size = egui::TextStyle::Button.resolve(ui.style()).size;
        let label_width = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    label.to_owned(),
                    egui::FontId::new(text_size, egui::FontFamily::Proportional),
                    ui.visuals().text_color(),
                )
                .size()
                .x
        });
        let button_width = (label_width + 52.0).clamp(118.0, 190.0).min(footer.width());
        let button_rect =
            egui::Rect::from_min_size(footer.left_top(), Vec2::new(button_width, footer_height));
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.put(
                    button_rect,
                    egui::Button::new("").min_size(Vec2::new(button_width, footer_height)),
                )
            })
            .inner;
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            response.rect,
            5.0,
            visuals.bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        paint_icon(
            ui,
            Icon::Refresh,
            egui::pos2(response.rect.left() + 17.0, response.rect.center().y),
            15.0,
            visuals.fg_stroke.color,
        );
        ui.painter().text(
            egui::pos2(response.rect.left() + 32.0, response.rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Button.resolve(ui.style()),
            visuals.fg_stroke.color,
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect.expand(1.0),
                visuals.corner_radius,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Outside,
            );
        }
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
        response.on_hover_text(tip).clicked()
    }
}

fn centered_message(ui: &egui::Ui, area: egui::Rect, message: &str) -> egui::Response {
    let color = Color32::from_gray(205);
    let painter = ui.painter().with_clip_rect(area);
    let galley = painter.layout(
        message.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        color,
        (area.width() - 40.0).max(1.0),
    );
    painter.galley(area.center() - galley.size() * 0.5, galley, color);
    ui.interact(area, ui.id().with("preview-message"), egui::Sense::hover())
}

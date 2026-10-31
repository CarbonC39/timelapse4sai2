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

    /// Draws a compact preview card. Returns true when the user requests a
    /// manual refresh; preview data remains owned by the caller.
    pub fn show(&mut self, ui: &mut egui::Ui, recording: bool) -> bool {
        const MAX_CARD: Vec2 = Vec2::new(360.0, 240.0);
        const PLACEHOLDER_CARD: Vec2 = Vec2::new(320.0, 180.0);
        const PAD: f32 = 16.0;

        let available_width = ui.available_width().max(0.0);
        let (card_size, image_size) = if let Some(texture) = &self.texture {
            let original = texture.size_vec2();
            if original.x > 0.0 && original.y > 0.0 {
                let image_bounds = Vec2::new(
                    (MAX_CARD.x - PAD * 2.0).min((available_width - PAD * 2.0).max(0.0)),
                    MAX_CARD.y - PAD * 2.0,
                );
                let scale = (image_bounds.x / original.x).min(image_bounds.y / original.y);
                let image_size = original * scale;
                (image_size + Vec2::splat(PAD * 2.0), Some(image_size))
            } else {
                (PLACEHOLDER_CARD, None)
            }
        } else {
            (PLACEHOLDER_CARD, None)
        };
        let card_size = Vec2::new(card_size.x.min(available_width), card_size.y);
        let row_size = Vec2::new(available_width, card_size.y);
        let (row, _) = ui.allocate_exact_size(row_size, egui::Sense::hover());
        let card = egui::Rect::from_center_size(row.center(), card_size);
        let painter = ui.painter().with_clip_rect(card);
        let well_color = if ui.visuals().dark_mode {
            Color32::from_rgb(43, 46, 50)
        } else {
            Color32::from_rgb(235, 236, 238)
        };
        painter.rect_filled(card, 8.0, well_color);

        if let (Some(texture), Some(image_size)) = (&self.texture, image_size) {
            let image = egui::Rect::from_center_size(card.center(), image_size);
            let image_painter = ui.painter().with_clip_rect(card.shrink(PAD));
            image_painter.image(
                texture.id(),
                image,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            if recording {
                image_painter.rect_filled(image, 0.0, Color32::from_black_alpha(155));
                image_painter.text(
                    image.center(),
                    egui::Align2::CENTER_CENTER,
                    tr(Key::Recording),
                    egui::TextStyle::Heading.resolve(ui.style()),
                    Color32::WHITE,
                );
            }
        } else {
            let inner = card.shrink(PAD);
            if self.loading {
                centered_message(ui, inner, tr(Key::Loading));
            } else if let Some(error) = &self.error {
                centered_message(ui, inner, tr(Key::PreviewUnavailable)).on_hover_text(error);
            } else {
                centered_message(ui, inner, tr(Key::PreviewEmpty));
            }
        }

        let refresh_rect = egui::Rect::from_min_size(
            egui::pos2(card.right() - 34.0, card.top() + 6.0),
            Vec2::splat(28.0),
        );
        let enabled = !self.loading;
        let refresh = ui.interact(
            refresh_rect,
            ui.id().with("preview-refresh"),
            if enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        let refresh_visuals = ui.style().interact(&refresh);
        ui.painter().rect(
            refresh.rect,
            5.0,
            refresh_visuals.bg_fill,
            refresh_visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        paint_icon(
            ui,
            Icon::Refresh,
            refresh.rect.center(),
            14.0,
            refresh_visuals.fg_stroke.color,
        );
        let refresh = refresh.on_hover_text(format!(
            "{}\n{}",
            tr(Key::RefreshPreview),
            tr(Key::PreviewRefreshTip)
        ));
        refresh.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tr(Key::RefreshPreview))
        });
        refresh.clicked()
    }
}

fn centered_message(ui: &egui::Ui, area: egui::Rect, message: &str) -> egui::Response {
    let color = if ui.visuals().dark_mode {
        Color32::from_gray(205)
    } else {
        Color32::from_gray(80)
    };
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

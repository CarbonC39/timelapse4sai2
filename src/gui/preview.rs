use super::{
    i18n::{Key, tr},
    widgets::{Icon, icon_button},
};
use eframe::egui::{self, Color32, Vec2};

pub struct Preview {
    texture: Option<egui::TextureHandle>,
    pub loading: bool,
    pub error: Option<String>,
    zoom: f32,
    pan: Vec2,
    pan_at_drag_start: Vec2,
    pan_mode: bool,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            texture: None,
            loading: false,
            error: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            pan_at_drag_start: Vec2::ZERO,
            pan_mode: false,
        }
    }
}

impl Preview {
    pub fn clear(&mut self) {
        self.texture = None;
        self.error = None;
        self.loading = false;
        self.reset_view();
    }

    pub fn set(
        &mut self,
        ctx: &egui::Context,
        result: Result<timelapse4sai2::preview::PreviewImage, String>,
    ) {
        self.loading = false;
        self.reset_view();
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

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.pan_at_drag_start = Vec2::ZERO;
        self.pan_mode = false;
    }

    /// Draw the preview and its controls. Returns true when the user requests a
    /// manual refresh; it deliberately does not read or rebuild the preview.
    pub fn show(&mut self, ui: &mut egui::Ui, recording: bool) -> bool {
        let mut refresh = false;
        let available = ui.available_size_before_wrap();
        let toolbar_height = ui.spacing().interact_size.y.max(28.0);
        let image_height = (available.y - toolbar_height - ui.spacing().item_spacing.y).max(0.0);
        let (image_area, image_response) = ui.allocate_exact_size(
            Vec2::new(available.x.max(0.0), image_height),
            if self.pan_mode && self.texture.is_some() && !self.loading {
                egui::Sense::drag()
            } else {
                egui::Sense::hover()
            },
        );

        if let Some(texture) = &self.texture {
            let original = texture.size_vec2();
            if original.x > 0.0 && original.y > 0.0 {
                let fit_scale = (image_area.width() / original.x)
                    .min(image_area.height() / original.y)
                    .max(0.01);
                let size = original * (fit_scale * self.zoom);
                let rect = egui::Rect::from_center_size(image_area.center() + self.pan, size);
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

                if self.pan_mode && self.zoom > 1.0 {
                    if image_response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    if image_response.drag_started() {
                        self.pan_at_drag_start = self.pan;
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    if image_response.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        self.pan = self.pan_at_drag_start + image_response.drag_delta();
                        let max_x = ((size.x - image_area.width()) * 0.5).max(0.0);
                        let max_y = ((size.y - image_area.height()) * 0.5).max(0.0);
                        self.pan.x = self.pan.x.clamp(-max_x, max_x);
                        self.pan.y = self.pan.y.clamp(-max_y, max_y);
                    }
                }
            }
        } else if self.loading {
            centered_message(ui, image_area, tr(Key::Loading));
        } else if let Some(error) = &self.error {
            let response = centered_message(ui, image_area, tr(Key::PreviewUnavailable));
            response.on_hover_text(error);
        } else {
            centered_message(ui, image_area, tr(Key::PreviewEmpty));
        }

        ui.allocate_ui_with_layout(
            Vec2::new(available.x.max(0.0), toolbar_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_height(toolbar_height);
                {
                    let button_size = ui.spacing().interact_size.y.max(28.0);
                    let group_width = button_size * 5.0 + ui.spacing().item_spacing.x * 4.0;
                    ui.add_space(((available.x - group_width) * 0.5).max(0.0));
                    if icon_button(ui, Icon::Refresh, tr(Key::PreviewRefreshTip), !self.loading)
                        .clicked()
                    {
                        refresh = true;
                    }

                    let has_image = self.texture.is_some() && !self.loading;
                    if icon_button(ui, Icon::ZoomIn, tr(Key::ZoomIn), has_image).clicked() {
                        self.zoom = (self.zoom * 1.25).min(8.0);
                    }
                    if icon_button(ui, Icon::ZoomOut, tr(Key::ZoomOut), has_image).clicked() {
                        self.zoom = (self.zoom / 1.25).max(0.25);
                        if self.zoom <= 1.0 {
                            self.pan = Vec2::ZERO;
                        }
                    }
                    if icon_button(ui, Icon::Fit, tr(Key::FitPreview), has_image).clicked() {
                        self.reset_view();
                    }
                    let move_response = icon_button(
                        ui,
                        Icon::Move,
                        tr(Key::MovePreview),
                        has_image && self.zoom > 1.0,
                    );
                    if move_response.clicked() {
                        self.pan_mode = !self.pan_mode;
                    }
                    if self.pan_mode && has_image {
                        ui.painter().rect_stroke(
                            move_response.rect.expand(1.0),
                            ui.visuals().widgets.active.corner_radius,
                            egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
            },
        );
        refresh
    }
}

fn centered_message(ui: &egui::Ui, area: egui::Rect, message: &str) -> egui::Response {
    let color = ui.visuals().weak_text_color();
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

use egui::{Color32, Pos2, Rect, Response, Stroke, Ui, Vec2};

/// Manages world-space <-> space transformation with pan and zoom.
#[derive(Clone, Debug)]
pub struct GridView {
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for GridView {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl GridView {
    pub fn world_to_screen(&self, world: Pos2, canvas_rect: Rect) -> Pos2 {
        let center = canvas_rect.center();
        Pos2::new(
            center.x + (world.x * self.zoom) + self.offset.x,
            center.y - (world.y * self.zoom) + self.offset.y,
        )
    }

    pub fn screen_to_world(&self, screen: Pos2, canvas_rect: Rect) -> Pos2 {
        let center = canvas_rect.center();
        Pos2::new(
            (screen.x - center.x - self.offset.x) / self.zoom,
            -(screen.y - center.y - self.offset.y) / self.zoom,
        )
    }

    /// Handle pan (drag) and zoom (scroll) input on the canvas.
    pub fn handle_input(&mut self, response: &Response) {
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary)
                && response.ctx.input(|i| i.modifiers.shift))
        {
            self.offset += response.drag_delta();
        }

        if response.hovered() {
            let scroll = response.ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = 1.0 + scroll * 0.002;
                self.zoom = (self.zoom * factor).clamp(0.01, 1000.0);
            }
        }
    }

    /// Draw the grid lines on the given canvas rect.
    pub fn draw(&self, ui: &mut Ui, canvas_rect: Rect) {
        let painter = ui.painter_at(canvas_rect);

        // Determine grid spacing based on zoom
        let base_spacing = compute_grid_spacing(self.zoom);

        // Draw grid lines
        let top_left = self.screen_to_world(canvas_rect.left_top(), canvas_rect);
        let bottom_right = self.screen_to_world(canvas_rect.right_bottom(), canvas_rect);

        let x_start = (top_left.x / base_spacing).floor() as i64;
        let x_end = (bottom_right.x / base_spacing).ceil() as i64;
        let y_start = (bottom_right.y / base_spacing).floor() as i64;
        let y_end = (top_left.y / base_spacing).ceil() as i64;

        let grid_color = Color32::from_gray(60);
        let axis_color = Color32::from_gray(120);
        let text_color = Color32::from_gray(160);

        for ix in x_start..=x_end {
            let wx = ix as f32 * base_spacing;
            let top = self.world_to_screen(Pos2::new(wx, top_left.y), canvas_rect);
            let bottom = self.world_to_screen(Pos2::new(wx, bottom_right.y), canvas_rect);
            let color = if ix == 0 { axis_color } else { grid_color };
            let width = if ix == 0 { 2.0 } else { 1.0 };
            painter.line_segment([top, bottom], Stroke::new(width, color));

            // Label
            if ix != 0 {
                let label_pos = self.world_to_screen(Pos2::new(wx, 0.0), canvas_rect);
                painter.text(
                    Pos2::new(label_pos.x + 2.0, label_pos.y + 2.0),
                    egui::Align2::LEFT_TOP,
                    format_grid_label(wx),
                    egui::FontId::proportional(10.0),
                    text_color,
                );
            }
        }

        for iy in y_start..=y_end {
            let wy = iy as f32 * base_spacing;
            let left = self.world_to_screen(Pos2::new(top_left.x, wy), canvas_rect);
            let right = self.world_to_screen(Pos2::new(bottom_right.x, wy), canvas_rect);
            let color = if iy == 0 { axis_color } else { grid_color };
            let width = if iy == 0 { 2.0 } else { 1.0 };
            painter.line_segment([left, right], Stroke::new(width, color));

            if iy != 0 {
                let label_pos = self.world_to_screen(Pos2::new(0.0, wy), canvas_rect);
                painter.text(
                    Pos2::new(label_pos.x + 2.0, label_pos.y - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    format_grid_label(wy),
                    egui::FontId::proportional(10.0),
                    text_color,
                );
            }
        }
    }
}

fn compute_grid_spacing(zoom: f32) -> f32 {
    let target_pixels = 80.0;
    let raw = target_pixels / zoom;
    let magnitude = 10.0_f32.powf(raw.log10().floor());
    let residual = raw / magnitude;
    let nice = if residual < 1.5 {
        1.0
    } else if residual < 3.5 {
        2.0
    } else if residual < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

fn format_grid_label(val: f32) -> String {
    if val.abs() >= 1000.0 {
        format!("{:.0}", val)
    } else if val.abs() >= 1.0 {
        format!("{:.1}", val)
    } else {
        format!("{:.2}", val)
    }
}

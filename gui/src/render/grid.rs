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
    /// Adjust zoom and offset so all nodes are visible with some margin.
    pub fn fit_to_nodes(&mut self, nodes: &[crate::state::NodeState], canvas_size: Vec2) {
        if nodes.is_empty() {
            return;
        }
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;
        let mut min_y = f64::MAX;
        let mut max_y = f64::MIN;
        for n in nodes {
            min_x = min_x.min(n.x);
            max_x = max_x.max(n.x);
            min_y = min_y.min(n.y);
            max_y = max_y.max(n.y);
        }
        let world_w = (max_x - min_x).max(1.0);
        let world_h = (max_y - min_y).max(1.0);
        let padding = 1.2; // 20% margin
        self.zoom = ((canvas_size.x as f64 / (world_w * padding))
            .min(canvas_size.y as f64 / (world_h * padding))) as f32;
        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;
        self.offset = Vec2::new(-(cx as f32) * self.zoom, (cy as f32) * self.zoom);
    }

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
    ///
    /// `drag_started_on_node` should be `true` when the pointer was over a node
    /// at drag start -- in that case primary-drag is suppressed to avoid panning
    /// when the user meant to click a node.
    pub fn handle_input(&mut self, response: &Response, drag_started_on_node: bool) {
        let middle_drag = response.dragged_by(egui::PointerButton::Middle);
        let shift_drag = response.dragged_by(egui::PointerButton::Primary)
            && response.ctx.input(|i| i.modifiers.shift);
        let primary_pan =
            response.dragged_by(egui::PointerButton::Primary) && !drag_started_on_node;

        if middle_drag || shift_drag || primary_pan {
            self.offset += response.drag_delta();
        }

        if response.hovered() {
            // Pinch-to-zoom on touchpad
            let pinch = response.ctx.input(|i| i.zoom_delta());
            if pinch != 1.0 {
                self.zoom = (self.zoom * pinch).clamp(0.01, 1000.0);
            }

            let ctrl_held = response.ctx.input(|i| i.modifiers.ctrl || i.modifiers.mac_cmd);
            let scroll = response.ctx.input(|i| i.smooth_scroll_delta);

            if ctrl_held && scroll.y != 0.0 {
                // Ctrl+MouseWheel = zoom
                let factor = 1.0 + scroll.y * 0.002;
                self.zoom = (self.zoom * factor).clamp(0.01, 1000.0);
            } else if scroll.x != 0.0 || scroll.y != 0.0 {
                // Plain scroll / two-finger swipe = pan
                self.offset += scroll;
            }
        }
    }

    /// Draw the grid lines and axis labels on the given canvas rect.
    pub fn draw(&self, ui: &mut Ui, canvas_rect: Rect, unit_label: &str) {
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
        let label_color = Color32::from_gray(200);

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

        // Axis labels in the bottom-right corner
        if !unit_label.is_empty() {
            let margin = 8.0;
            let font = egui::FontId::proportional(12.0);
            // X-axis label
            painter.text(
                Pos2::new(canvas_rect.right() - margin, canvas_rect.bottom() - margin),
                egui::Align2::RIGHT_BOTTOM,
                format!("X ({unit_label})"),
                font.clone(),
                label_color,
            );
            // Y-axis label
            painter.text(
                Pos2::new(canvas_rect.left() + margin, canvas_rect.top() + margin),
                egui::Align2::LEFT_TOP,
                format!("Y ({unit_label})"),
                font,
                label_color,
            );
        }
    }

    /// Compute the visible world-space bounding box.
    fn world_bounds(&self, canvas_rect: Rect) -> (Pos2, Pos2) {
        let tl = self.screen_to_world(canvas_rect.left_top(), canvas_rect);
        let br = self.screen_to_world(canvas_rect.right_bottom(), canvas_rect);
        (
            Pos2::new(tl.x.min(br.x), br.y.min(tl.y)),
            Pos2::new(tl.x.max(br.x), br.y.max(tl.y)),
        )
    }

    /// Compute content bounding box from nodes with margin.
    fn content_bounds(nodes: &[crate::state::NodeState]) -> Option<(Pos2, Pos2)> {
        if nodes.is_empty() {
            return None;
        }
        let mut cmin = Pos2::new(f32::MAX, f32::MAX);
        let mut cmax = Pos2::new(f32::MIN, f32::MIN);
        for n in nodes {
            cmin.x = cmin.x.min(n.x as f32);
            cmin.y = cmin.y.min(n.y as f32);
            cmax.x = cmax.x.max(n.x as f32);
            cmax.y = cmax.y.max(n.y as f32);
        }
        let margin = ((cmax.x - cmin.x).max(cmax.y - cmin.y)).max(1.0) * 0.5;
        cmin.x -= margin;
        cmin.y -= margin;
        cmax.x += margin;
        cmax.y += margin;
        Some((cmin, cmax))
    }

    /// Interactive scrollbars on all four edges of the canvas.
    ///
    /// Horizontal bars appear near the top and bottom edges; vertical bars near
    /// the left and right edges. They become visible when the cursor is within
    /// `PROXIMITY` pixels of the edge and can be dragged to pan the view.
    ///
    /// Returns `true` if a scrollbar is currently being dragged.
    pub fn handle_scrollbars(
        &mut self,
        ui: &mut Ui,
        canvas_rect: Rect,
        nodes: &[crate::state::NodeState],
    ) -> bool {
        let Some((content_min, content_max)) = Self::content_bounds(nodes) else {
            return false;
        };
        let content_w = content_max.x - content_min.x;
        let content_h = content_max.y - content_min.y;
        if content_w <= 0.0 || content_h <= 0.0 {
            return false;
        }

        let pointer_pos = ui.ctx().input(|i| i.pointer.hover_pos());
        let pointer_down = ui.ctx().input(|i| i.pointer.primary_down());
        let pointer_pressed = ui.ctx().input(|i| i.pointer.primary_pressed());
        let drag_delta = ui.ctx().input(|i| i.pointer.delta());

        const PROXIMITY: f32 = 40.0;
        const BAR_THICKNESS: f32 = 8.0;
        const BAR_INSET: f32 = 3.0;
        const ROUNDING: f32 = 4.0;

        // Persistent drag state
        let drag_id = ui.id().with("scrollbar_drag");
        let mut dragging: Option<ScrollbarEdge> =
            ui.data(|d| d.get_temp(drag_id)).unwrap_or(None);

        if !pointer_down {
            dragging = None;
        }

        let painter = ui.painter_at(canvas_rect);
        let mut any_dragging = false;

        // Process each edge
        for &edge in &[
            ScrollbarEdge::Bottom,
            ScrollbarEdge::Top,
            ScrollbarEdge::Right,
            ScrollbarEdge::Left,
        ] {
            let is_horizontal = matches!(edge, ScrollbarEdge::Top | ScrollbarEdge::Bottom);

            // Compute thumb fractions
            let (view_min, view_max) = self.world_bounds(canvas_rect);
            let (frac_start, frac_end) = if is_horizontal {
                let fl = ((view_min.x - content_min.x) / content_w).clamp(0.0, 1.0);
                let fr = ((view_max.x - content_min.x) / content_w).clamp(0.0, 1.0);
                (fl, fr)
            } else {
                // Screen Y inverted vs world Y
                let ft = ((content_max.y - view_max.y) / content_h).clamp(0.0, 1.0);
                let fb = ((content_max.y - view_min.y) / content_h).clamp(0.0, 1.0);
                (ft, fb)
            };

            // Compute geometry
            let (track_rect, thumb_rect) = edge.geometry(
                canvas_rect,
                frac_start,
                frac_end,
                BAR_THICKNESS,
                BAR_INSET,
            );

            let near_edge = pointer_pos.is_some_and(|p| {
                let expanded = track_rect.expand2(if is_horizontal {
                    Vec2::new(0.0, PROXIMITY)
                } else {
                    Vec2::new(PROXIMITY, 0.0)
                });
                expanded.contains(p)
            });

            let is_this_dragging = dragging == Some(edge);

            // Start drag on press (not just down) to avoid re-triggering
            if pointer_pressed && dragging.is_none() {
                if let Some(p) = pointer_pos {
                    if thumb_rect.expand(4.0).contains(p) {
                        dragging = Some(edge);
                    } else if near_edge && track_rect.contains(p) {
                        // Click on track: jump thumb center to click position
                        if is_horizontal {
                            let click_frac =
                                (p.x - canvas_rect.left()) / canvas_rect.width();
                            let center_frac = (frac_start + frac_end) / 2.0;
                            let world_dx = (click_frac - center_frac) * content_w;
                            self.offset.x -= world_dx * self.zoom;
                        } else {
                            let click_frac =
                                (p.y - canvas_rect.top()) / canvas_rect.height();
                            let center_frac = (frac_start + frac_end) / 2.0;
                            let world_dy = (click_frac - center_frac) * content_h;
                            self.offset.y -= world_dy * self.zoom;
                        }
                        dragging = Some(edge);
                    }
                }
            }

            // Apply drag delta: dragging scrollbar right/down moves viewport right/down
            if is_this_dragging {
                if is_horizontal && drag_delta.x != 0.0 {
                    let frac_dx = drag_delta.x / canvas_rect.width();
                    self.offset.x += frac_dx * content_w * self.zoom;
                } else if !is_horizontal && drag_delta.y != 0.0 {
                    let frac_dy = drag_delta.y / canvas_rect.height();
                    self.offset.y += frac_dy * content_h * self.zoom;
                }
                any_dragging = true;
            }

            // Draw when near or dragging
            if near_edge || is_this_dragging {
                let alpha = if is_this_dragging { 180 } else { 100 };
                let color = Color32::from_rgba_premultiplied(150, 150, 150, alpha);

                // Recompute thumb after potential offset change
                let (vm, vx) = self.world_bounds(canvas_rect);
                let (fs, fe) = if is_horizontal {
                    (
                        ((vm.x - content_min.x) / content_w).clamp(0.0, 1.0),
                        ((vx.x - content_min.x) / content_w).clamp(0.0, 1.0),
                    )
                } else {
                    (
                        ((content_max.y - vx.y) / content_h).clamp(0.0, 1.0),
                        ((content_max.y - vm.y) / content_h).clamp(0.0, 1.0),
                    )
                };
                let (_, updated_thumb) =
                    edge.geometry(canvas_rect, fs, fe, BAR_THICKNESS, BAR_INSET);
                painter.rect_filled(updated_thumb, ROUNDING, color);
            }
        }

        ui.data_mut(|d| d.insert_temp(drag_id, dragging));

        if any_dragging {
            ui.ctx().request_repaint();
        }

        any_dragging
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ScrollbarEdge {
    Top,
    Bottom,
    Left,
    Right,
}

impl ScrollbarEdge {
    /// Returns (track_rect, thumb_rect) for this edge.
    fn geometry(
        self,
        canvas: Rect,
        frac_start: f32,
        frac_end: f32,
        thickness: f32,
        inset: f32,
    ) -> (Rect, Rect) {
        match self {
            Self::Bottom => {
                let y = canvas.bottom() - inset - thickness;
                let track = Rect::from_min_max(
                    Pos2::new(canvas.left(), y),
                    Pos2::new(canvas.right(), y + thickness),
                );
                let thumb = Rect::from_min_max(
                    Pos2::new(
                        canvas.left() + frac_start * canvas.width(),
                        y,
                    ),
                    Pos2::new(
                        canvas.left() + frac_end * canvas.width(),
                        y + thickness,
                    ),
                );
                (track, thumb)
            }
            Self::Top => {
                let y = canvas.top() + inset;
                let track = Rect::from_min_max(
                    Pos2::new(canvas.left(), y),
                    Pos2::new(canvas.right(), y + thickness),
                );
                let thumb = Rect::from_min_max(
                    Pos2::new(
                        canvas.left() + frac_start * canvas.width(),
                        y,
                    ),
                    Pos2::new(
                        canvas.left() + frac_end * canvas.width(),
                        y + thickness,
                    ),
                );
                (track, thumb)
            }
            Self::Right => {
                let x = canvas.right() - inset - thickness;
                let track = Rect::from_min_max(
                    Pos2::new(x, canvas.top()),
                    Pos2::new(x + thickness, canvas.bottom()),
                );
                let thumb = Rect::from_min_max(
                    Pos2::new(x, canvas.top() + frac_start * canvas.height()),
                    Pos2::new(
                        x + thickness,
                        canvas.top() + frac_end * canvas.height(),
                    ),
                );
                (track, thumb)
            }
            Self::Left => {
                let x = canvas.left() + inset;
                let track = Rect::from_min_max(
                    Pos2::new(x, canvas.top()),
                    Pos2::new(x + thickness, canvas.bottom()),
                );
                let thumb = Rect::from_min_max(
                    Pos2::new(x, canvas.top() + frac_start * canvas.height()),
                    Pos2::new(
                        x + thickness,
                        canvas.top() + frac_end * canvas.height(),
                    ),
                );
                (track, thumb)
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

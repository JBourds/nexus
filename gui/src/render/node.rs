use egui::{Color32, Pos2, Rect, Stroke, Vec2, Ui};

use crate::render::grid::GridView;
use crate::state::NodeState;

const NODE_RADIUS: f32 = 4.0;

/// Compute the display radius of a node at the given zoom level.
pub fn node_radius(zoom: f32) -> f32 {
    NODE_RADIUS * zoom.sqrt().clamp(0.3, 3.0)
}

/// Draw a single node on the canvas.
pub fn draw_node(
    ui: &mut Ui,
    canvas_rect: Rect,
    grid: &GridView,
    node: &NodeState,
    selected: bool,
) {
    let world_pos = Pos2::new(node.x as f32, node.y as f32);
    let screen_pos = grid.world_to_screen(world_pos, canvas_rect);

    if !canvas_rect.contains(screen_pos) {
        return;
    }

    let painter = ui.painter_at(canvas_rect);
    let color = charge_color(node.charge_ratio);
    let radius = node_radius(grid.zoom);

    // Node circle
    painter.circle_filled(screen_pos, radius, color);

    // Dead node X overlay
    if node.is_dead {
        let half = radius * 0.7;
        let stroke = Stroke::new(2.0, Color32::RED);
        painter.line_segment([screen_pos + Vec2::new(-half, -half), screen_pos + Vec2::new(half, half)], stroke);
        painter.line_segment([screen_pos + Vec2::new(-half, half), screen_pos + Vec2::new(half, -half)], stroke);
    }

    // Selection ring
    if selected {
        painter.circle_stroke(screen_pos, radius + 3.0, Stroke::new(2.0, Color32::WHITE));
    }

    // Label
    painter.text(
        Pos2::new(screen_pos.x, screen_pos.y - radius - 4.0),
        egui::Align2::CENTER_BOTTOM,
        &node.name,
        egui::FontId::proportional(11.0),
        Color32::from_gray(220),
    );
}

/// Map charge ratio to a color: green (100%) → yellow (50%) → red (0%).
/// Blue if no charge tracking.
fn charge_color(charge_ratio: Option<f32>) -> Color32 {
    match charge_ratio {
        None => Color32::from_rgb(80, 140, 220),
        Some(0.0) => Color32::from_rgba_premultiplied(80, 80, 80, 128),
        Some(r) => {
            let r = r.clamp(0.0, 1.0);
            if r > 0.5 {
                let t = (r - 0.5) * 2.0;
                Color32::from_rgb((255.0 * (1.0 - t)) as u8, 200, (50.0 * (1.0 - t)) as u8)
            } else {
                let t = r * 2.0;
                Color32::from_rgb(255, (200.0 * t) as u8, 0)
            }
        }
    }
}

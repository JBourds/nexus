use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};

use crate::constants::*;
use crate::render::grid::GridView;
use crate::state::NodeState;

/// Compute the display radius of a node at the given zoom level.
pub fn node_radius(zoom: f32) -> f32 {
    NODE_RADIUS * zoom.sqrt().clamp(NODE_ZOOM_CLAMP_MIN, NODE_ZOOM_CLAMP_MAX)
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
        painter.line_segment(
            [
                screen_pos + Vec2::new(-half, -half),
                screen_pos + Vec2::new(half, half),
            ],
            stroke,
        );
        painter.line_segment(
            [
                screen_pos + Vec2::new(-half, half),
                screen_pos + Vec2::new(half, -half),
            ],
            stroke,
        );
    }

    // Selection ring
    if selected {
        painter.circle_stroke(
            screen_pos,
            radius + NODE_SELECTION_RING_OFFSET,
            Stroke::new(2.0, Color32::WHITE),
        );
    }

    // Label
    painter.text(
        Pos2::new(screen_pos.x, screen_pos.y - radius - NODE_LABEL_OFFSET),
        egui::Align2::CENTER_BOTTOM,
        &node.name,
        egui::FontId::proportional(NODE_LABEL_FONT_SIZE),
        COLOR_HEADER,
    );
}

/// Draw a colored highlight ring around a node (for receiver expansion).
pub fn draw_node_highlight(
    ui: &mut Ui,
    canvas_rect: Rect,
    grid: &GridView,
    node: &NodeState,
    color: Color32,
) {
    let world_pos = Pos2::new(node.x as f32, node.y as f32);
    let screen_pos = grid.world_to_screen(world_pos, canvas_rect);
    if !canvas_rect.contains(screen_pos) {
        return;
    }
    let painter = ui.painter_at(canvas_rect);
    let radius = node_radius(grid.zoom);
    painter.circle_stroke(
        screen_pos,
        radius + NODE_HIGHLIGHT_RING_OFFSET,
        Stroke::new(2.5, color),
    );
}

/// Map charge ratio to a color: green (100%) -> yellow (50%) -> red (0%).
/// Blue if no charge tracking.
fn charge_color(charge_ratio: Option<f32>) -> Color32 {
    match charge_ratio {
        None => COLOR_NODE_DEFAULT,
        Some(0.0) => COLOR_NODE_DEAD,
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

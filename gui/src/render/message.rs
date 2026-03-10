use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};

use crate::render::grid::GridView;
use crate::state::{ArrowKind, NodeState};

/// Draw an in-flight message arc between two nodes with an arrowhead.
pub fn draw_message_arc(
    ui: &mut Ui,
    canvas_rect: Rect,
    grid: &GridView,
    src: &NodeState,
    dst: &NodeState,
    progress: f32,
    kind: ArrowKind,
) {
    let painter = ui.painter_at(canvas_rect);
    let src_screen = grid.world_to_screen(Pos2::new(src.x as f32, src.y as f32), canvas_rect);
    let dst_screen = grid.world_to_screen(Pos2::new(dst.x as f32, dst.y as f32), canvas_rect);

    let (line_color, dot_color) = match kind {
        ArrowKind::Sent => (
            Color32::from_rgba_premultiplied(100, 200, 100, 60),
            Color32::from_rgb(100, 200, 100),
        ),
        ArrowKind::Received => (
            Color32::from_rgba_premultiplied(100, 150, 255, 60),
            Color32::from_rgb(100, 150, 255),
        ),
        ArrowKind::Dropped => (
            Color32::from_rgba_premultiplied(255, 100, 100, 60),
            Color32::from_rgb(255, 100, 100),
        ),
    };

    // Draw line from src to dst
    painter.line_segment([src_screen, dst_screen], Stroke::new(1.5, line_color));

    // Draw moving dot at progress position
    let dot_pos = Pos2::new(
        src_screen.x + (dst_screen.x - src_screen.x) * progress,
        src_screen.y + (dst_screen.y - src_screen.y) * progress,
    );
    painter.circle_filled(dot_pos, 4.0, dot_color);

    // Arrowhead at destination
    if progress > 0.9 {
        let dir = Vec2::new(dst_screen.x - src_screen.x, dst_screen.y - src_screen.y);
        let len = dir.length();
        if len > 1.0 {
            let norm = dir / len;
            let perp = Vec2::new(-norm.y, norm.x);
            let tip = dst_screen;
            let arrow_len = 8.0;
            let arrow_width = 4.0;
            let base = tip - norm * arrow_len;
            let left = base + perp * arrow_width;
            let right = base - perp * arrow_width;
            painter.add(egui::Shape::convex_polygon(
                vec![tip, left, right],
                dot_color,
                Stroke::NONE,
            ));
        }
    }

    // Drop X at midpoint
    if kind == ArrowKind::Dropped {
        let mid = Pos2::new(
            (src_screen.x + dst_screen.x) / 2.0,
            (src_screen.y + dst_screen.y) / 2.0,
        );
        let half = 5.0;
        let stroke = Stroke::new(2.0, dot_color);
        painter.line_segment(
            [mid + Vec2::new(-half, -half), mid + Vec2::new(half, half)],
            stroke,
        );
        painter.line_segment(
            [mid + Vec2::new(-half, half), mid + Vec2::new(half, -half)],
            stroke,
        );
    }
}

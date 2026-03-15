use egui::{Pos2, Rect, Stroke, Ui, Vec2};

use crate::constants::*;
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
        ArrowKind::Sent => (COLOR_TX_OK_TRAIL, COLOR_TX_OK),
        ArrowKind::Received => (COLOR_RX_TRAIL, COLOR_RX),
        ArrowKind::Dropped => (COLOR_DROP_TRAIL, COLOR_DROP),
    };

    // Draw line from src to dst
    painter.line_segment(
        [src_screen, dst_screen],
        Stroke::new(ARROW_LINE_WIDTH, line_color),
    );

    // Draw moving dot at progress position
    let dot_pos = Pos2::new(
        src_screen.x + (dst_screen.x - src_screen.x) * progress,
        src_screen.y + (dst_screen.y - src_screen.y) * progress,
    );
    painter.circle_filled(dot_pos, ARROW_DOT_RADIUS, dot_color);

    // Arrowhead at destination
    if progress > ARROW_HEAD_THRESHOLD {
        let dir = Vec2::new(dst_screen.x - src_screen.x, dst_screen.y - src_screen.y);
        let len = dir.length();
        if len > 1.0 {
            let norm = dir / len;
            let perp = Vec2::new(-norm.y, norm.x);
            let tip = dst_screen;
            let base = tip - norm * ARROW_HEAD_LENGTH;
            let left = base + perp * ARROW_HEAD_WIDTH;
            let right = base - perp * ARROW_HEAD_WIDTH;
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
        let half = ARROW_DROP_X_HALF;
        let stroke = Stroke::new(ARROW_DROP_X_STROKE, dot_color);
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

use egui::{Color32, Pos2, Rect, Stroke, Ui};

use crate::render::grid::GridView;
use crate::state::NodeState;

/// Draw an in-flight message arc between two nodes.
pub fn draw_message_arc(
    ui: &mut Ui,
    canvas_rect: Rect,
    grid: &GridView,
    src: &NodeState,
    dst: &NodeState,
    progress: f32,
) {
    let painter = ui.painter_at(canvas_rect);
    let src_screen = grid.world_to_screen(Pos2::new(src.x as f32, src.y as f32), canvas_rect);
    let dst_screen = grid.world_to_screen(Pos2::new(dst.x as f32, dst.y as f32), canvas_rect);

    // Draw dashed line
    let color = Color32::from_rgba_premultiplied(255, 200, 50, 180);
    painter.line_segment(
        [src_screen, dst_screen],
        Stroke::new(1.5, color.linear_multiply(0.3)),
    );

    // Draw moving dot at progress position
    let dot_pos = Pos2::new(
        src_screen.x + (dst_screen.x - src_screen.x) * progress,
        src_screen.y + (dst_screen.y - src_screen.y) * progress,
    );
    painter.circle_filled(dot_pos, 4.0, color);
}

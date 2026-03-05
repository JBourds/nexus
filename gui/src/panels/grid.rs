use egui::{Pos2, Rect, Sense, Ui};

use crate::render;
use crate::render::grid::GridView;
use crate::state::NodeState;

/// Draw the central canvas with grid and nodes.
/// Returns (clicked_node, hovered_node).
pub fn show_grid_panel(
    ui: &mut Ui,
    grid: &mut GridView,
    nodes: &[NodeState],
    selected_node: &Option<String>,
) -> (Option<String>, Option<String>) {
    let available = ui.available_size();
    let (canvas_rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());

    // Handle pan/zoom
    grid.handle_input(&response);

    // Draw grid
    grid.draw(ui, canvas_rect);

    // Draw nodes (rendering only)
    for node in nodes {
        let is_selected = selected_node.as_ref().is_some_and(|s| s == &node.name);
        render::node::draw_node(ui, canvas_rect, grid, node, is_selected);
    }

    // Detect clicked and hovered nodes using the canvas response and pointer position.
    // We use the canvas response for clicks (not raw input) because the canvas widget
    // with Sense::click_and_drag() consumes pointer events — raw any_click() is unreliable.
    let clicked_node = if response.clicked() {
        response
            .interact_pointer_pos()
            .and_then(|pos| hit_test_node(pos, canvas_rect, grid, nodes))
    } else {
        None
    };

    let hovered_node = ui
        .ctx()
        .input(|i| i.pointer.hover_pos())
        .and_then(|pos| hit_test_node(pos, canvas_rect, grid, nodes));

    (clicked_node, hovered_node)
}

/// Find which node (if any) is at the given screen position.
fn hit_test_node(
    pos: Pos2,
    canvas_rect: Rect,
    grid: &GridView,
    nodes: &[NodeState],
) -> Option<String> {
    for node in nodes {
        let world_pos = Pos2::new(node.x as f32, node.y as f32);
        let screen_pos = grid.world_to_screen(world_pos, canvas_rect);
        let radius = render::node::node_radius(grid.zoom);
        let node_rect = Rect::from_center_size(screen_pos, egui::Vec2::splat(radius * 2.0));
        if node_rect.contains(pos) {
            return Some(node.name.clone());
        }
    }
    None
}

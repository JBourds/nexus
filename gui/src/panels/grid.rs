use egui::{Sense, Ui};

use crate::render;
use crate::render::grid::GridView;
use crate::state::NodeState;

/// Draw the central canvas with grid and nodes. Returns the name of a clicked node, if any.
pub fn show_grid_panel(
    ui: &mut Ui,
    grid: &mut GridView,
    nodes: &[NodeState],
    selected_node: &Option<String>,
) -> Option<String> {
    let available = ui.available_size();
    let (canvas_rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());

    // Handle pan/zoom
    grid.handle_input(&response);

    // Draw grid
    grid.draw(ui, canvas_rect);

    // Draw nodes
    let mut clicked_node = None;
    for node in nodes {
        let is_selected = selected_node.as_ref().is_some_and(|s| s == &node.name);
        if render::node::draw_node(ui, canvas_rect, grid, node, is_selected) {
            clicked_node = Some(node.name.clone());
        }
    }

    clicked_node
}

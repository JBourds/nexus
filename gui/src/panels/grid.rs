use config::ast::DistanceUnit;
use egui::{Pos2, Rect, Sense, Ui};

use crate::render;
use crate::render::grid::GridView;
use crate::state::{ArrowAnimation, NodeState};

/// Draw the central canvas with grid and nodes.
/// Returns (clicked_node, hovered_node).
pub fn show_grid_panel(
    ui: &mut Ui,
    grid: &mut GridView,
    nodes: &[NodeState],
    selected_node: &Option<String>,
    arrows: &[ArrowAnimation],
    distance_unit: DistanceUnit,
) -> (Option<String>, Option<String>) {
    let unit_label = distance_unit_abbrev(distance_unit);
    let available = ui.available_size();
    let (canvas_rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());

    // Track whether drag started on a node (persisted across frames via egui temp data).
    let drag_on_node_id = ui.id().with("drag_on_node");
    let mut drag_started_on_node: bool = ui.data(|d| d.get_temp(drag_on_node_id)).unwrap_or(false);
    if response.drag_started() {
        drag_started_on_node = response
            .interact_pointer_pos()
            .and_then(|pos| hit_test_node(pos, canvas_rect, grid, nodes))
            .is_some();
        ui.data_mut(|d| d.insert_temp(drag_on_node_id, drag_started_on_node));
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.insert_temp(drag_on_node_id, false));
    }

    // Handle pan/zoom
    grid.handle_input(&response, drag_started_on_node);

    // Draw grid
    grid.draw(ui, canvas_rect, unit_label);

    // Draw nodes (rendering only)
    for node in nodes {
        let is_selected = selected_node.as_ref().is_some_and(|s| s == &node.name);
        render::node::draw_node(ui, canvas_rect, grid, node, is_selected);
    }

    // Draw active message arrows
    let egui_time = ui.ctx().input(|i| i.time);
    for arrow in arrows {
        if let (Some(src), Some(dst)) = (nodes.get(arrow.src_node), nodes.get(arrow.dst_node)) {
            let progress =
                ((egui_time - arrow.start_time) / arrow.duration as f64).clamp(0.0, 1.0) as f32;
            render::message::draw_message_arc(
                ui,
                canvas_rect,
                grid,
                src,
                dst,
                progress,
                arrow.kind,
            );
        }
    }

    // Detect clicked and hovered nodes using the canvas response and pointer position.
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

fn distance_unit_abbrev(unit: DistanceUnit) -> &'static str {
    match unit {
        DistanceUnit::Millimeters => "mm",
        DistanceUnit::Centimeters => "cm",
        DistanceUnit::Meters => "m",
        DistanceUnit::Kilometers => "km",
    }
}

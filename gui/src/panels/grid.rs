use std::collections::HashMap;

use config::ast::DistanceUnit;
use egui::{Color32, Pos2, Rect, Sense, Ui};

use crate::render;
use crate::render::grid::GridView;
use crate::state::{ArrowAnimation, NodeState};

/// Draw the central canvas with grid and nodes.
/// Returns (clicked_node, hovered_node).
///
/// `node_highlights` maps node name -> ring color (e.g. green for received, red for dropped).
/// `draggable` enables click-and-drag repositioning of nodes (only in config editor).
#[allow(clippy::too_many_arguments)]
pub fn show_grid_panel(
    ui: &mut Ui,
    grid: &mut GridView,
    nodes: &mut [NodeState],
    selected_node: &Option<String>,
    arrows: &[ArrowAnimation],
    distance_unit: DistanceUnit,
    node_highlights: &HashMap<String, Color32>,
    draggable: bool,
) -> (Option<String>, Option<String>) {
    let unit_label = distance_unit_abbrev(distance_unit);
    let available = ui.available_size();
    let (canvas_rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());

    // Track which node the drag started on (persisted across frames via egui temp data).
    let drag_on_node_id = ui.id().with("drag_on_node");
    let dragged_node_id = ui.id().with("dragged_node_name");
    let mut drag_started_on_node: bool = ui.data(|d| d.get_temp(drag_on_node_id)).unwrap_or(false);
    if response.drag_started() {
        let hit = response
            .interact_pointer_pos()
            .and_then(|pos| hit_test_node(pos, canvas_rect, grid, nodes));
        drag_started_on_node = draggable && hit.is_some();
        ui.data_mut(|d| d.insert_temp(drag_on_node_id, drag_started_on_node));
        ui.data_mut(|d| d.insert_temp::<Option<String>>(dragged_node_id, hit));
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.insert_temp(drag_on_node_id, false));
        ui.data_mut(|d| d.insert_temp::<Option<String>>(dragged_node_id, None));
    }

    // Node dragging: move the dragged node by the screen delta converted to world coords.
    let is_shift = response.ctx.input(|i| i.modifiers.shift);
    if drag_started_on_node && !is_shift && response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        if delta.x != 0.0 || delta.y != 0.0 {
            let dragged_name: Option<String> =
                ui.data(|d| d.get_temp(dragged_node_id)).unwrap_or(None);
            if let Some(ref name) = dragged_name
                && let Some(node) = nodes.iter_mut().find(|n| &n.name == name)
            {
                // Convert screen delta to world delta (Y is inverted in world coords)
                node.x += (delta.x / grid.zoom) as f64;
                node.y -= (delta.y / grid.zoom) as f64;
            }
        }
    }

    // Interactive scrollbars (must come before handle_input so drag is detected first)
    let scrollbar_active = grid.handle_scrollbars(ui, canvas_rect, nodes);

    // Handle pan/zoom (suppressed when dragging a scrollbar)
    if !scrollbar_active {
        grid.handle_input(&response, drag_started_on_node);
    }

    // Draw grid
    grid.draw(ui, canvas_rect, unit_label);

    // Draw nodes (rendering only)
    for node in nodes.iter() {
        let is_selected = selected_node.as_ref().is_some_and(|s| s == &node.name);
        render::node::draw_node(ui, canvas_rect, grid, node, is_selected);
        // Draw receiver highlight ring if applicable
        if let Some(&color) = node_highlights.get(&node.name) {
            render::node::draw_node_highlight(ui, canvas_rect, grid, node, color);
        }
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

    // Show grab cursor when hovering over a draggable node
    if draggable {
        if drag_started_on_node && response.dragged_by(egui::PointerButton::Primary) && !is_shift {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if hovered_node.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }

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

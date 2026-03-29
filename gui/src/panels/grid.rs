use std::collections::HashMap;

use config::ast::DistanceUnit;
use config::terrain::{ObstacleShape, TerrainMap};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui};

use crate::render;
use crate::render::grid::GridView;
use crate::state::{ArrowAnimation, NodeState, TerrainOverlay};

/// Draw the central canvas with grid and nodes.
/// Returns (clicked_node, hovered_node).
///
/// `node_highlights` maps node name -> ring color (e.g. green for received, red for dropped).
pub fn show_grid_panel(
    ui: &mut Ui,
    grid: &mut GridView,
    nodes: &[NodeState],
    selected_node: &Option<String>,
    arrows: &[ArrowAnimation],
    distance_unit: DistanceUnit,
    node_highlights: &HashMap<String, Color32>,
    terrain: Option<&TerrainMap>,
    mut terrain_overlay: Option<&mut TerrainOverlay>,
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

    // Interactive scrollbars (must come before handle_input so drag is detected first)
    let scrollbar_active = grid.handle_scrollbars(ui, canvas_rect, nodes);

    // Handle pan/zoom (suppressed when dragging a scrollbar)
    if !scrollbar_active {
        grid.handle_input(&response, drag_started_on_node);
    }

    // Initialize terrain overlay textures if needed.
    if let Some(t) = terrain {
        if let Some(ref mut overlay) = terrain_overlay {
            overlay.init_from_terrain(t, ui.ctx());
        }
    }

    // Draw terrain background layer (before grid lines and nodes).
    if let Some(ref overlay) = terrain_overlay {
        draw_terrain_overlay(ui, canvas_rect, grid, overlay);
    }

    // Draw obstacle outlines.
    if let Some(t) = terrain {
        draw_obstacle_outlines(ui, canvas_rect, grid, t);
    }

    // Draw grid
    grid.draw(ui, canvas_rect, unit_label);

    // Draw nodes (rendering only)
    for node in nodes {
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

    (clicked_node, hovered_node)
}

/// Draw the terrain overlay (heightmap or map image) as a background layer.
fn draw_terrain_overlay(
    ui: &Ui,
    canvas_rect: Rect,
    grid: &GridView,
    overlay: &TerrainOverlay,
) {
    let painter = ui.painter_at(canvas_rect);

    // Draw heightmap texture if available (used as fallback when no map_image).
    if overlay.map_texture.is_none() {
        if let Some(ref tex) = overlay.heightmap_texture {
            let world_min = Pos2::new(overlay.hm_bounds_min.0 as f32, overlay.hm_bounds_max.1 as f32);
            let world_max = Pos2::new(overlay.hm_bounds_max.0 as f32, overlay.hm_bounds_min.1 as f32);
            let screen_min = grid.world_to_screen(world_min, canvas_rect);
            let screen_max = grid.world_to_screen(world_max, canvas_rect);
            let rect = Rect::from_min_max(
                Pos2::new(screen_min.x.min(screen_max.x), screen_min.y.min(screen_max.y)),
                Pos2::new(screen_min.x.max(screen_max.x), screen_min.y.max(screen_max.y)),
            );
            let tint = Color32::from_white_alpha(180);
            painter.image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                tint,
            );
        }
    }

    // Draw map overlay image if available (drawn on top of heightmap).
    if let Some(ref tex) = overlay.map_texture {
        let world_min = Pos2::new(overlay.map_bounds_min.0 as f32, overlay.map_bounds_max.1 as f32);
        let world_max = Pos2::new(overlay.map_bounds_max.0 as f32, overlay.map_bounds_min.1 as f32);
        let screen_min = grid.world_to_screen(world_min, canvas_rect);
        let screen_max = grid.world_to_screen(world_max, canvas_rect);
        let rect = Rect::from_min_max(
            Pos2::new(screen_min.x.min(screen_max.x), screen_min.y.min(screen_max.y)),
            Pos2::new(screen_min.x.max(screen_max.x), screen_min.y.max(screen_max.y)),
        );
        let alpha = (overlay.map_opacity * 255.0) as u8;
        let tint = Color32::from_white_alpha(alpha);
        painter.image(
            tex.id(),
            rect,
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            tint,
        );
    }
}

/// Draw semi-transparent outlines for terrain obstacles.
fn draw_obstacle_outlines(
    ui: &Ui,
    canvas_rect: Rect,
    grid: &GridView,
    terrain: &TerrainMap,
) {
    let painter = ui.painter_at(canvas_rect);
    let fill_color = Color32::from_rgba_premultiplied(200, 80, 80, 40);
    let stroke = Stroke::new(1.5, Color32::from_rgba_premultiplied(200, 80, 80, 120));

    for obs in terrain.obstacles() {
        match obs.shape() {
            ObstacleShape::Rect { min, max } => {
                // World Y is inverted relative to screen Y.
                let screen_min = grid.world_to_screen(
                    Pos2::new(min[0] as f32, max[1] as f32),
                    canvas_rect,
                );
                let screen_max = grid.world_to_screen(
                    Pos2::new(max[0] as f32, min[1] as f32),
                    canvas_rect,
                );
                let rect = Rect::from_min_max(
                    Pos2::new(screen_min.x.min(screen_max.x), screen_min.y.min(screen_max.y)),
                    Pos2::new(screen_min.x.max(screen_max.x), screen_min.y.max(screen_max.y)),
                );
                painter.rect(rect, 0.0, fill_color, stroke, egui::StrokeKind::Outside);
            }
            ObstacleShape::Line {
                start,
                end,
                thickness,
            } => {
                // Expand line to a thin rect for visualization.
                let half = *thickness as f32 / 2.0;
                let min_x = (start[0] as f32).min(end[0] as f32) - half;
                let min_y = (start[1] as f32).min(end[1] as f32) - half;
                let max_x = (start[0] as f32).max(end[0] as f32) + half;
                let max_y = (start[1] as f32).max(end[1] as f32) + half;
                let screen_min = grid.world_to_screen(
                    Pos2::new(min_x, max_y),
                    canvas_rect,
                );
                let screen_max = grid.world_to_screen(
                    Pos2::new(max_x, min_y),
                    canvas_rect,
                );
                let rect = Rect::from_min_max(
                    Pos2::new(screen_min.x.min(screen_max.x), screen_min.y.min(screen_max.y)),
                    Pos2::new(screen_min.x.max(screen_max.x), screen_min.y.max(screen_max.y)),
                );
                painter.rect(rect, 0.0, fill_color, stroke, egui::StrokeKind::Outside);
            }
        }
    }
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

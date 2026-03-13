use egui::Color32;

// -- Window -------------------------------------------------------------------
pub const WINDOW_INITIAL_SIZE: [f32; 2] = [1200.0, 800.0];
pub const WINDOW_MIN_SIZE: [f32; 2] = [800.0, 600.0];

// -- Palette (no raw hex) -----------------------------------------------------
pub const COLOR_TX_OK: Color32 = Color32::from_rgb(100, 200, 100);
pub const COLOR_RX: Color32 = Color32::from_rgb(100, 150, 255);
pub const COLOR_DROP: Color32 = Color32::from_rgb(255, 100, 100);
pub const COLOR_BIT_ERR: Color32 = Color32::from_rgb(255, 210, 60);
pub const COLOR_LIFELINE: Color32 = Color32::from_gray(60);
pub const COLOR_HEADER: Color32 = Color32::from_gray(220);
pub const COLOR_TS_LABEL: Color32 = Color32::from_gray(100);
pub const COLOR_HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(255, 255, 100, 15);

// Arrow trail colors (translucent versions of TX/RX/DROP)
pub const COLOR_TX_OK_TRAIL: Color32 = Color32::from_rgba_premultiplied(100, 200, 100, 60);
pub const COLOR_RX_TRAIL: Color32 = Color32::from_rgba_premultiplied(100, 150, 255, 60);
pub const COLOR_DROP_TRAIL: Color32 = Color32::from_rgba_premultiplied(255, 100, 100, 60);

// Event highlight (inspector, messages panels)
pub const COLOR_EVENT_HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(255, 255, 100, 30);

// Node colors
pub const COLOR_NODE_DEFAULT: Color32 = Color32::from_rgb(80, 140, 220);
pub const COLOR_NODE_DEAD: Color32 = Color32::from_rgba_premultiplied(80, 80, 80, 128);
pub const COLOR_MOTION_SPEC: Color32 = Color32::from_rgb(180, 180, 255);

// Breakpoints / run-until
pub const COLOR_BREAKPOINT_ENABLED: Color32 = Color32::from_rgb(255, 80, 80);
pub const COLOR_BREAKPOINT_DISABLED: Color32 = Color32::from_gray(120);
pub const COLOR_RUN_UNTIL: Color32 = Color32::from_rgb(255, 200, 80);

// Labels / text (dim variants)
pub const COLOR_LABEL_DIM: Color32 = Color32::from_gray(160);
pub const COLOR_LABEL_DIMMER: Color32 = Color32::from_gray(140);

// Grid canvas
pub const COLOR_GRID_LINE: Color32 = Color32::from_gray(60);
pub const COLOR_GRID_AXIS: Color32 = Color32::from_gray(120);
pub const COLOR_GRID_TEXT: Color32 = Color32::from_gray(160);
pub const COLOR_GRID_LABEL: Color32 = Color32::from_gray(200);

// Config editor
pub const COLOR_REMOVE_BUTTON: Color32 = Color32::from_rgb(200, 60, 60);
pub const COLOR_MODULE_REMOVE: Color32 = Color32::from_rgb(220, 60, 60);
pub const COLOR_IMPORTED_GREEN: Color32 = Color32::from_rgb(60, 160, 60);

// -- Node rendering -----------------------------------------------------------
pub const NODE_RADIUS: f32 = 4.0;
pub const NODE_ZOOM_CLAMP_MIN: f32 = 0.3;
pub const NODE_ZOOM_CLAMP_MAX: f32 = 3.0;
pub const NODE_SELECTION_RING_OFFSET: f32 = 3.0;
pub const NODE_HIGHLIGHT_RING_OFFSET: f32 = 5.0;
pub const NODE_LABEL_FONT_SIZE: f32 = 11.0;
pub const NODE_LABEL_OFFSET: f32 = 4.0;

// -- Arrow animation ----------------------------------------------------------
pub const ARROW_DURATION: f32 = 0.25;
pub const ARROW_DOT_RADIUS: f32 = 4.0;
pub const ARROW_LINE_WIDTH: f32 = 1.5;
pub const ARROW_HEAD_LENGTH: f32 = 8.0;
pub const ARROW_HEAD_WIDTH: f32 = 4.0;
pub const ARROW_HEAD_THRESHOLD: f32 = 0.9;
pub const ARROW_DROP_X_HALF: f32 = 5.0;
pub const ARROW_DROP_X_STROKE: f32 = 2.0;

// -- Sequence diagram ---------------------------------------------------------
pub const SEQ_BASE_ROW_HEIGHT: f32 = 24.0;
pub const SEQ_BASE_LIFELINE_SPACING: f32 = 100.0;
pub const SEQ_HEADER_HEIGHT: f32 = 30.0;
pub const SEQ_TS_LABEL_MARGIN: f32 = 50.0;
pub const SEQ_ZOOM_MIN: f32 = 0.15;
pub const SEQ_ZOOM_MAX: f32 = 5.0;
pub const SEQ_FONT_SIZE_BASE: f32 = 12.0;
pub const SEQ_FONT_SIZE_MIN: f32 = 7.0;
pub const SEQ_FONT_SIZE_MAX: f32 = 18.0;
pub const SEQ_TS_FONT_BASE: f32 = 9.0;
pub const SEQ_TS_FONT_MIN: f32 = 6.0;
pub const SEQ_TS_FONT_MAX: f32 = 14.0;
pub const SEQ_LIFELINE_DASH: f32 = 6.0;
pub const SEQ_LIFELINE_GAP: f32 = 4.0;
pub const SEQ_LIFELINE_STROKE: f32 = 1.0;
pub const SEQ_ARROW_HEAD_LENGTH: f32 = 6.0;
pub const SEQ_ARROW_HEAD_WIDTH: f32 = 3.0;
pub const SEQ_DROP_X_HALF: f32 = 4.0;
pub const SEQ_DROP_X_STROKE: f32 = 2.0;
pub const SEQ_HOVER_RECT_SIZE: f32 = 12.0;
pub const SEQ_RX_DASH: f32 = 3.0;
pub const SEQ_RX_GAP: f32 = 2.0;
pub const SEQ_RX_SEG_HALF_FACTOR: f32 = 0.35;
pub const SEQ_BOTTOM_PADDING: f32 = 20.0;

// -- Grid view ----------------------------------------------------------------
pub const GRID_ZOOM_MIN: f32 = 0.01;
pub const GRID_ZOOM_MAX: f32 = 1000.0;
pub const GRID_SCROLL_ZOOM_FACTOR: f32 = 0.002;
pub const GRID_TARGET_PIXEL_SPACING: f32 = 80.0;
pub const GRID_FIT_PADDING: f64 = 1.2;
pub const GRID_LABEL_FONT_SIZE: f32 = 10.0;
pub const GRID_AXIS_LABEL_FONT_SIZE: f32 = 12.0;
pub const GRID_AXIS_LABEL_MARGIN: f32 = 8.0;
pub const GRID_AXIS_WIDTH: f32 = 2.0;
pub const GRID_LINE_WIDTH: f32 = 1.0;
pub const GRID_LABEL_OFFSET: f32 = 2.0;

// -- Layout -------------------------------------------------------------------
pub const PANEL_FRAME_MARGIN: f32 = 6.0;
pub const CONFIG_PANEL_WIDTH: f32 = 300.0;
pub const INSPECTOR_PANEL_WIDTH: f32 = 180.0;
pub const BREAKPOINTS_PANEL_WIDTH: f32 = 220.0;
pub const MAX_MESSAGES_DISPLAY: usize = 200;
pub const INSPECTOR_EVENTS_SCROLL_HEIGHT: f32 = 200.0;
pub const BREAKPOINTS_SCROLL_HEIGHT: f32 = 120.0;

// -- Playback -----------------------------------------------------------------
pub const PLAYBACK_SPEED_MIN: f32 = 0.1;
pub const PLAYBACK_SPEED_MAX: f32 = 10.0;
pub const PLAYBACK_SPEED_DEFAULT: f32 = 1.0;
pub const SEQ_ZOOM_DEFAULT: f32 = 1.0;

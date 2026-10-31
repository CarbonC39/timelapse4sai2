#[derive(Debug, Clone, Copy)]
pub struct CanvasLayout {
    pub next_offset: usize,
    pub id_offset: Option<usize>,
    pub width_offset: usize,
    pub height_offset: usize,
    pub name_offset: usize,
    pub short_path_offset: usize,
    pub string_bytes: usize,
    pub tiles: TileLayout,
}

#[derive(Debug, Clone, Copy)]
pub struct TileLayout {
    /// Inline array entry for level 0: canvas -> slot -> map -> row tree.
    pub level0_offset: usize,
    pub canvas_grid_offsets: Option<(usize, usize)>,
    pub tree_offset: usize,
    pub width_offset: usize,
    pub height_offset: usize,
    pub nx_offset: usize,
    pub ny_offset: usize,
}

pub mod legacy_preview;
pub mod renovated_alpha;

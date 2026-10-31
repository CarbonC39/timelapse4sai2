use super::{CanvasLayout, TileLayout};

pub const LAYOUT: CanvasLayout = CanvasLayout {
    next_offset: 0,
    id_offset: None,
    width_offset: 0x28,
    height_offset: 0x2c,
    name_offset: 0x8f8,
    short_path_offset: 0xb00,
    string_bytes: 520,
    // Upstream sai.py SAICanvas: inline array of 11 RP(RP(TileMap)) entries.
    // Level 0 still requires two reads; the grid on the canvas is independent.
    tiles: TileLayout {
        level0_offset: 0x48,
        canvas_grid_offsets: Some((0x180, 0x184)),
        tree_offset: 8,
        width_offset: 0x10,
        height_offset: 0x14,
        nx_offset: 0x18,
        ny_offset: 0x1c,
    },
};

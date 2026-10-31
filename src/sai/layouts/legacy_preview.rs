use super::{CanvasLayout, TileLayout};

pub const LAYOUT: CanvasLayout = CanvasLayout {
    next_offset: 0,
    id_offset: Some(0x20),
    width_offset: 0x250,
    height_offset: 0x254,
    name_offset: 0x2ac,
    short_path_offset: 0x4ac,
    string_bytes: 512,
    tiles: TileLayout {
        level0_offset: 0x28,
        canvas_grid_offsets: None,
        tree_offset: 8,
        width_offset: 0x10,
        height_offset: 0x14,
        nx_offset: 0x18,
        ny_offset: 0x1c,
    },
};

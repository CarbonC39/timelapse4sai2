pub const SESSION_OFFSET: usize = 0x32eb20;
pub const LAYOUT: super::reader::Layout = super::reader::Layout {
    session_offset: SESSION_OFFSET,
    canvas_id_offset: 0x20,
    canvas_tile_map_slot_offset: 0x28,
    canvas_width_offset: 0x250,
    canvas_height_offset: 0x254,
    canvas_name_offset: 0x2ac,
    canvas_short_path_offset: 0x4ac,
    map_tree_offset: 8,
    map_width_offset: 0x10,
    map_height_offset: 0x14,
    map_nx_offset: 0x18,
    map_ny_offset: 0x1c,
    pointer_stride: 8,
};
pub const MD5: &str = "6f3f351303cf3896f4c2977925c994c2";
pub const MAX_DIMENSION: u32 = 100_000;
pub const MAX_RGB_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    pub address: usize,
    pub id: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub name: String,
    pub short_path: String,
}

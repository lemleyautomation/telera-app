#[derive(Debug, Default)]
pub enum Shapes {
    #[default]
    Circle,
    Line{width: f32}
}
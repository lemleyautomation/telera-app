#[derive(Debug, Default)]
pub enum LineDirection{
    Horizontal,
    #[default]
    Vertical,
}

#[derive(Debug, Default)]
pub enum Shapes {
    #[default]
    Circle,
    Line{width: f32}
}
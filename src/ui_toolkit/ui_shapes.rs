use symbol_table::GlobalSymbol;

//use crate::DataSrc;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct LineConfig{
    pub width_source: Option<GlobalSymbol>,
    pub width: f32
}

// impl LineConfig {
//     pub fn clone_static_width(&self, width: f32) -> LineConfig {
//         let mut newc = self.clone();
//         newc.width = DataSrc::Static(width);
//         newc
//     }
//     pub fn clone_dynamic_width(&self, width: GlobalSymbol) -> LineConfig {
//         let mut newc = self.clone();
//         newc.width = DataSrc::Dynamic(width);
//         newc
//     }
// }

#[derive(Debug, Default, Clone, PartialEq)]
pub enum CustomElement {
    #[default]
    Circle,
    Line(LineConfig)
}

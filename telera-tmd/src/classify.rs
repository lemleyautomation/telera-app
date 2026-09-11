//! Rust field-type classification.
//!
//! `#[derive(LayoutRunnerReflection)]` / `#[derive(FieldAccess)]` expose a
//! struct's fields to a `.tmd` layout **by name**, and a field's Rust type alone
//! decides *which* slot it can fill:
//!
//! | Rust field type (last path-segment ident) | [`FieldKind`] | TML slot |
//! |---|---|---|
//! | `bool` | `Bool` | `if` / `if-not` condition |
//! | `u8`…`i128`, `usize`, `isize`, `f32`, `f64` | `Numeric` | any numeric value slot |
//! | `String` | `Text` | a `text` element's content |
//! | `Color` | `Color` | `color` / `font-color` / `border-color` |
//! | `UIImageDescriptor` | `Image` | `image` config |
//! | `Vec<T>` | `List(_)` | `list` / `item` iteration |
//! | anything else | (not classified) | not referenceable |
//!
//! Only the **last** path segment's identifier is inspected - `telera_app::Color`
//! and `Color` both classify as `Color` - and a path written with a leading `::`
//! is rejected. Type aliases are not resolved (there is no type information at
//! this layer); this exactly matches what the derive macros themselves see, so
//! the language server and the runtime agree on which names resolve.
//!
//! Field names are matched **verbatim** everywhere - no case folding, no
//! space/hyphen/underscore equivalence.

use syn::{Attribute, GenericArgument, Ident, PathArguments, Type};

/// What a field's Rust type tells us to expose it as.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Bool,
    Numeric,
    Text,
    Color,
    Image,
    /// A `Vec<T>`. Carries the element type's own [`FieldKind`] when `T` is a
    /// primitive we can expose directly (`Vec<String>`, `Vec<f32>`, `Vec<bool>`,
    /// `Vec<Color>`, `Vec<UIImageDescriptor>`); `None` when `T` is a struct that
    /// has to provide its fields through `T: FieldAccess`.
    List(Option<Box<FieldKind>>),
}

/// Classify a `syn::Type` per the table in the module docs. `None` for anything
/// a `.tmd` layout cannot reference.
pub fn classify_field_type(ty: &Type) -> Option<FieldKind> {
    let Type::Path(path) = ty else {
        return None;
    };
    if path.path.leading_colon.is_some() {
        return None;
    }
    let segment = path.path.segments.last()?;
    match segment.ident.to_string().as_str() {
        "bool" => Some(FieldKind::Bool),
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" | "f32" | "f64" => Some(FieldKind::Numeric),
        "String" => Some(FieldKind::Text),
        "Color" => Some(FieldKind::Color),
        "UIImageDescriptor" => Some(FieldKind::Image),
        "Vec" => {
            let PathArguments::AngleBracketed(args) = &segment.arguments else {
                return None;
            };
            let Some(GenericArgument::Type(inner)) = args.args.first() else {
                return None;
            };
            Some(FieldKind::List(classify_field_type(inner).map(Box::new)))
        }
        _ => None,
    }
}

/// The last path-segment identifier of a `syn::Type`, e.g. `Doc` for
/// `Vec<crate::model::Doc>`. Used to link a `Vec<T>` field to the
/// `#[derive(FieldAccess)]` struct that provides its per-item names.
pub fn last_type_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => Some(path.path.segments.last()?.ident.to_string()),
        _ => None,
    }
}

/// The element type of a `Vec<T>` field, as a `&Type` (for further inspection),
/// or `None` if `ty` isn't a `Vec<_>`.
pub fn vec_element_type(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != "Vec" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

/// Find `#[name]` / `#[name(...)]` on a field (single-segment path only).
pub fn find_attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs
        .iter()
        .find(|attribute| attribute.path().segments.len() == 1 && attribute.path().is_ident(name))
}

pub fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    find_attr(attrs, name).is_some()
}

/// Parse `#[name(some_ident)]`'s single identifier argument. Returns `None` when
/// the attribute isn't shaped that way (the derive macro `panic!`s in that
/// case; the language server prefers to report a diagnostic).
pub fn attr_ident_arg(attr: &Attribute) -> Option<Ident> {
    attr.parse_args::<Ident>().ok()
}

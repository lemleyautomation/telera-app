use syn::{GenericArgument, Ident, PathArguments, Type};

// ---------------------------------------------------------------------------
// ParserDataAccess / FieldAccess
//
// Both derives expose a struct's own fields as `get-*`/`field_*` lookups by
// name, so a layout can reference `self.some_field` as `*some field*`
// without the application writing that plumbing by hand. A field's Rust
// type alone decides which lookup it's exposed through:
//
//   bool                                -> get_bool     / field_bool
//   u8/u16/../i8/../f32/f64/usize/isize -> get_numeric  / field_numeric
//   String                              -> get_text     / field_text
//   Color                               -> get_color    / field_color
//   Vec<T>                              -> get_list_length (ParserDataAccess only)
//
// A markdown name is matched against a field's Rust name after normalizing
// both sides the same way (see `normalize_field_symbol`): lowercase, with
// spaces/hyphens folded to underscores. That lets `*content background
// color*` match `content_background_color` and `file-menu-opened` match
// `file_menu_open` without the field needing an un-idiomatic Rust name.
//
// A `Vec<T>` field additionally participates in list lookups (`list Name`
// in the markdown) three ways, controlled by attributes on the field:
//
//   (default)              the current list index's item's own fields are
//                           looked up via `T: FieldAccess` - so `T` should
//                           `#[derive(FieldAccess)]`, or this won't compile.
//   #[no_field_access]      skip that - `get_list_length` only.
//   #[list_click_event(name)]  `left-clicked *Clicked*` inside this list
//                           resolves to the handler name `"name"`, dispatched
//                           through `LayoutReflector::dispatch_event`.
//
// Anything index-shaped - "is this the selected row", "show me item N's
// fields outside of any list" - is a *markdown* concern, not a Rust one: see
// `item`/`if-index`/`if-index-not` in `layout_runner.rs`. They're built on
// exactly the same `Vec<T>` + `FieldAccess` plumbing above, so a plain
// unattributed index field (e.g. `selected_document: usize`) is all a
// struct needs to provide.
// ---------------------------------------------------------------------------

/// What a field's Rust type tells us to expose it as.
enum FieldKind {
    Bool,
    Numeric,
    Text,
    Color,
    /// A `Vec<T>`; the element type's identifier (e.g. `Document`).
    List(Ident),
}

fn classify_field_type(ty: &Type) -> Option<FieldKind> {
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
        "Vec" => {
            let PathArguments::AngleBracketed(args) = &segment.arguments else {
                return None;
            };
            let Some(GenericArgument::Type(Type::Path(inner))) = args.args.first() else {
                return None;
            };
            Some(FieldKind::List(inner.path.segments.last()?.ident.clone()))
        }
        _ => None,
    }
}

fn find_attr<'a>(attrs: &'a [syn::Attribute], name: &str) -> Option<&'a syn::Attribute> {
    attrs
        .iter()
        .find(|attribute| attribute.path().segments.len() == 1 && attribute.path().is_ident(name))
}

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    find_attr(attrs, name).is_some()
}

/// Parses `#[name(some_ident)]`'s single identifier argument.
fn attr_ident_arg(attr: &syn::Attribute) -> Ident {
    attr.parse_args::<Ident>().unwrap_or_else(|_| {
        let name = attr
            .path()
            .get_ident()
            .map(|i| i.to_string())
            .unwrap_or_default();
        panic!("`#[{name}(...)]` needs a single identifier argument, e.g. `#[{name}(documents)]`")
    })
}

#[proc_macro_derive(LayoutRunnerReflection, attributes(list_click_event, no_field_access))]
pub fn parser_data_acces(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast: syn::DeriveInput = syn::parse(item).unwrap();
    let struct_name = ast.ident.clone();

    let mut plain_bool = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_numeric = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_text = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_color = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_list_length = Vec::<proc_macro2::TokenStream>::new();

    let mut list_bool = Vec::<proc_macro2::TokenStream>::new();
    let mut list_numeric = Vec::<proc_macro2::TokenStream>::new();
    let mut list_text = Vec::<proc_macro2::TokenStream>::new();
    let mut list_color = Vec::<proc_macro2::TokenStream>::new();
    let mut list_event = Vec::<proc_macro2::TokenStream>::new();

    if let syn::Data::Struct(data) = &ast.data {
        for field in &data.fields {
            let Some(field_ident) = &field.ident else {
                continue;
            };
            let field_name = field_ident.to_string();
            let Some(kind) = classify_field_type(&field.ty) else {
                continue;
            };

            match kind {
                FieldKind::Bool => {
                    plain_bool.push(quote::quote! {
                        #field_name => return Some(self.#field_ident),
                    });
                }
                FieldKind::Numeric => {
                    plain_numeric.push(quote::quote! {
                        #field_name => return Some(self.#field_ident as f32),
                    });
                }
                FieldKind::Text => {
                    plain_text.push(quote::quote! {
                        #field_name => return Some(&self.#field_ident),
                    });
                }
                FieldKind::Color => {
                    plain_color.push(quote::quote! {
                        #field_name => return Some(&self.#field_ident),
                    });
                }
                FieldKind::List(_element_type) => {
                    plain_list_length.push(quote::quote! {
                        #field_name => Some(self.#field_ident.len()),
                    });

                    if let Some(attr) = find_attr(&field.attrs, "list_click_event") {
                        let handler_name = attr_ident_arg(attr).to_string();
                        list_event.push(quote::quote! {
                            if list_key == #field_name && key == "clicked" {
                                return Some(symbol_table::GlobalSymbol::new(#handler_name));
                            }
                        });
                    }

                    if !has_attr(&field.attrs, "no_field_access") {
                        list_bool.push(quote::quote! {
                            if list_key == #field_name
                                && let Some(item) = self.#field_ident.get(*index)
                                && let Some(value) = telera_app::FieldAccess::field_bool(item, name)
                            {
                                return Some(value);
                            }
                        });
                        list_numeric.push(quote::quote! {
                            if list_key == #field_name
                                && let Some(item) = self.#field_ident.get(*index)
                                && let Some(value) = telera_app::FieldAccess::field_numeric(item, name)
                            {
                                return Some(value);
                            }
                        });
                        list_text.push(quote::quote! {
                            if list_key == #field_name
                                && let Some(item) = self.#field_ident.get(*index)
                                && let Some(value) = telera_app::FieldAccess::field_text(item, name)
                            {
                                return Some(value);
                            }
                        });
                        list_color.push(quote::quote! {
                            if list_key == #field_name
                                && let Some(item) = self.#field_ident.get(*index)
                                && let Some(value) = telera_app::FieldAccess::field_color(item, name)
                            {
                                return Some(value);
                            }
                        });
                    }
                }
            }
        }
    } else {
        panic!("#[derive(LayoutRunnerReflection)] can only be used on structs");
    }

    quote::quote! {
        impl LayoutRunnerReflection for #struct_name {
            #[allow(unused_variables)]
            fn get_bool(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<bool> {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_bool)*
                    _ => {}
                }
                if let Some((list_symbol, index)) = list_data {
                    let list_key = telera_app::normalize_field_symbol(list_symbol.as_str());
                    #(#list_bool)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_numeric(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<f32> {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_numeric)*
                    _ => {}
                }
                if let Some((list_symbol, index)) = list_data {
                    let list_key = telera_app::normalize_field_symbol(list_symbol.as_str());
                    #(#list_numeric)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_text<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<&'render_pass String>
            where
                'application: 'render_pass,
            {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_text)*
                    _ => {}
                }
                if let Some((list_symbol, index)) = list_data {
                    let list_key = telera_app::normalize_field_symbol(list_symbol.as_str());
                    #(#list_text)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_color<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<&'render_pass telera_app::Color>
            where
                'application: 'render_pass,
            {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_color)*
                    _ => {}
                }
                if let Some((list_symbol, index)) = list_data {
                    let list_key = telera_app::normalize_field_symbol(list_symbol.as_str());
                    #(#list_color)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_event<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<symbol_table::GlobalSymbol>
            where
                'application: 'render_pass,
            {
                let key = telera_app::normalize_field_symbol(name.as_str());
                if let Some((list_symbol, index)) = list_data {
                    let list_key = telera_app::normalize_field_symbol(list_symbol.as_str());
                    #(#list_event)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_list_length(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<usize> {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_list_length)*
                    _ => None,
                }
            }
        }
    }.into()
}

/// Companion to `#[derive(ParserDataAccess)]` for a `Vec<T>` field's element
/// type `T`, so a `list` in a layout can resolve each item's own fields.
/// See the module-level notes above `ParserDataAccess` for the type-to-getter
/// mapping; this covers the non-list-aware subset of it (`bool`/numeric/
/// `String`/`Color` fields only - a `Vec` field on an item type is just
/// ignored, since nested lists aren't supported).
#[proc_macro_derive(FieldAccess)]
pub fn field_access(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast: syn::DeriveInput = syn::parse(item).unwrap();
    let struct_name = ast.ident.clone();

    let mut plain_bool = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_numeric = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_text = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_color = Vec::<proc_macro2::TokenStream>::new();

    if let syn::Data::Struct(data) = &ast.data {
        for field in &data.fields {
            let Some(field_ident) = &field.ident else {
                continue;
            };
            let field_name = field_ident.to_string();
            match classify_field_type(&field.ty) {
                Some(FieldKind::Bool) => plain_bool.push(quote::quote! {
                    #field_name => return Some(self.#field_ident),
                }),
                Some(FieldKind::Numeric) => plain_numeric.push(quote::quote! {
                    #field_name => return Some(self.#field_ident as f32),
                }),
                Some(FieldKind::Text) => plain_text.push(quote::quote! {
                    #field_name => return Some(&self.#field_ident),
                }),
                Some(FieldKind::Color) => plain_color.push(quote::quote! {
                    #field_name => return Some(&self.#field_ident),
                }),
                Some(FieldKind::List(_)) | None => {}
            }
        }
    } else {
        panic!("#[derive(FieldAccess)] can only be used on structs");
    }

    quote::quote! {
        impl telera_app::FieldAccess for #struct_name {
            #[allow(unused_variables)]
            fn field_bool(&self, name: &symbol_table::GlobalSymbol) -> Option<bool> {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_bool)*
                    _ => None,
                }
            }
            #[allow(unused_variables)]
            fn field_numeric(&self, name: &symbol_table::GlobalSymbol) -> Option<f32> {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_numeric)*
                    _ => None,
                }
            }
            #[allow(unused_variables)]
            fn field_text(&self, name: &symbol_table::GlobalSymbol) -> Option<&String> {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_text)*
                    _ => None,
                }
            }
            #[allow(unused_variables)]
            fn field_color(&self, name: &symbol_table::GlobalSymbol) -> Option<&telera_app::Color> {
                let key = telera_app::normalize_field_symbol(name.as_str());
                match key.as_str() {
                    #(#plain_color)*
                    _ => None,
                }
            }
        }
    }
    .into()
}

#[proc_macro_derive(App)]
pub fn app(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast: syn::DeriveInput = syn::parse(item).unwrap();
    let struct_name = ast.ident.clone();

    quote::quote! {
        impl App for #struct_name {}
    }
    .into()
}

/// `#[telera_app]` goes on an application's inherent `impl` block and writes
/// its [`LayoutReflector`] impl for it, wiring each method tagged with a
/// marker attribute into the right dispatcher:
///
/// * `#[layout_event]` - `fn name(&mut self, context: Option<EventContext>,
///   api: &mut API<Self>)` - reachable from a layout config like
///   `left-clicked name`, dispatched through `dispatch_event`.
/// * `#[layout_element]` - `fn name(&mut self, api: &mut API<Self>, mt: &mut
///   MT)` - reachable from a `fn *name*` element, dispatched through
///   `dispatch_custom_element`.
///
/// The markdown always refers to a method by its Rust name. Methods without
/// either marker are left alone, and the marker attributes are stripped from
/// the emitted `impl` so the untagged methods compile normally.
#[proc_macro_attribute]
pub fn telera_app(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut item_impl = syn::parse_macro_input!(item as syn::ItemImpl);
    let self_ty = item_impl.self_ty.clone();

    let mut event_arms = Vec::<proc_macro2::TokenStream>::new();
    let mut element_arms = Vec::<proc_macro2::TokenStream>::new();

    for impl_item in &mut item_impl.items {
        let syn::ImplItem::Fn(method) = impl_item else {
            continue;
        };

        let is_event = method.attrs.iter().any(|a| a.path().is_ident("layout_event"));
        let is_element = method
            .attrs
            .iter()
            .any(|a| a.path().is_ident("layout_element"));

        // Strip the markers - they're not real attributes, so the re-emitted
        // method has to shed them to compile.
        method
            .attrs
            .retain(|a| !a.path().is_ident("layout_event") && !a.path().is_ident("layout_element"));

        if is_event && is_element {
            return syn::Error::new_spanned(
                &method.sig.ident,
                "a method can be `#[layout_event]` or `#[layout_element]`, not both",
            )
            .to_compile_error()
            .into();
        }

        let name = method.sig.ident.clone();
        let name_str = name.to_string();

        if is_event {
            event_arms.push(quote::quote! {
                #name_str => self.#name(context, api),
            });
        } else if is_element {
            element_arms.push(quote::quote! {
                #name_str => self.#name(api, mt),
            });
        }
    }

    quote::quote! {
        #item_impl

        impl telera_app::LayoutReflector<#self_ty> for #self_ty {
            #[allow(unused_variables)]
            fn dispatch_event(
                &mut self,
                name: &symbol_table::GlobalSymbol,
                context: ::core::option::Option<telera_app::EventContext>,
                api: &mut telera_app::API<#self_ty>,
            ) {
                match name.as_str() {
                    #(#event_arms)*
                    _ => {}
                }
            }

            #[allow(unused_variables)]
            fn dispatch_custom_element(
                &mut self,
                name: &symbol_table::GlobalSymbol,
                api: &mut telera_app::API<#self_ty>,
                mt: &mut telera_app::MT,
            ) {
                match name.as_str() {
                    #(#element_arms)*
                    _ => {}
                }
            }
        }
    }
    .into()
}

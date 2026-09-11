use telera_tmd::classify::{FieldKind, classify_field_type, find_attr, has_attr};

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
//   UIImageDescriptor                   -> get_image    / field_image
//   Vec<T>                              -> get_list_length (ParserDataAccess only)
//
// A markdown name is matched against a field's Rust name **exactly** - no
// case folding, no space/hyphen/underscore equivalence. `*content_background_color*`
// matches a `content_background_color` field; `*content background color*` or
// `*Content-Background-Color*` do not. Write the Rust identifier verbatim.
//
// A `Vec<T>` field additionally participates in list lookups (`list Name`
// in the markdown) three ways, controlled by attributes on the field:
//
//   (default)              the current list index's item's own fields are
//                           looked up via `T: FieldAccess` - so `T` should
//                           `#[derive(FieldAccess)]`, or this won't compile.
//   #[no_field_access]      skip that - `get_list_length` only.
//   #[list_click_event(name)]  `left-clicked *clicked*` inside this list
//                           (the arg spelled exactly `clicked`) resolves to the
//                           handler name `"name"`, dispatched through
//                           `LayoutReflector::dispatch_event`.
//
// Anything index-shaped - "is this the selected row", "show me item N's
// fields outside of any list" - is a *markdown* concern, not a Rust one: see
// `item`/`if-index`/`if-index-not` in `layout_runner.rs`. They're built on
// exactly the same `Vec<T>` + `FieldAccess` plumbing above, so a plain
// unattributed index field (e.g. `selected_document: usize`) is all a
// struct needs to provide.
// ---------------------------------------------------------------------------

/// Parses `#[name(some_ident)]`'s single identifier argument.
///
/// [`telera_tmd::classify::attr_ident_arg`] returns `None` when the attribute
/// isn't shaped that way (the language server reports a diagnostic); the derive
/// macros keep the old panicking contract.
fn attr_ident_arg(attr: &syn::Attribute) -> syn::Ident {
    telera_tmd::classify::attr_ident_arg(attr).unwrap_or_else(|| {
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
    let mut plain_image = Vec::<proc_macro2::TokenStream>::new();
    let mut plain_list_length = Vec::<proc_macro2::TokenStream>::new();

    let mut list_bool = Vec::<proc_macro2::TokenStream>::new();
    let mut list_numeric = Vec::<proc_macro2::TokenStream>::new();
    let mut list_text = Vec::<proc_macro2::TokenStream>::new();
    let mut list_color = Vec::<proc_macro2::TokenStream>::new();
    let mut list_image = Vec::<proc_macro2::TokenStream>::new();
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
                        if *name == symbol_table::static_symbol!(#field_name) { return Some(self.#field_ident); }
                    });
                }
                FieldKind::Numeric => {
                    plain_numeric.push(quote::quote! {
                        if *name == symbol_table::static_symbol!(#field_name) { return Some(self.#field_ident as f32); }
                    });
                }
                FieldKind::Text => {
                    plain_text.push(quote::quote! {
                        if *name == symbol_table::static_symbol!(#field_name) { return Some(&self.#field_ident); }
                    });
                }
                FieldKind::Color => {
                    plain_color.push(quote::quote! {
                        if *name == symbol_table::static_symbol!(#field_name) { return Some(&self.#field_ident); }
                    });
                }
                FieldKind::Image => {
                    plain_image.push(quote::quote! {
                        if *name == symbol_table::static_symbol!(#field_name) { return Some(&self.#field_ident); }
                    });
                }
                FieldKind::List(element_kind) => {
                    plain_list_length.push(quote::quote! {
                        if *name == symbol_table::static_symbol!(#field_name) { return Some(self.#field_ident.len()); }
                    });

                    if let Some(attr) = find_attr(&field.attrs, "list_click_event") {
                        let handler_name = attr_ident_arg(attr).to_string();
                        list_event.push(quote::quote! {
                            if *list_symbol == symbol_table::static_symbol!(#field_name) && *name == symbol_table::static_symbol!("clicked") {
                                return Some(symbol_table::GlobalSymbol::new(#handler_name));
                            }
                        });
                    }

                    if has_attr(&field.attrs, "no_field_access") {
                        // Only `get_list_length` for this field.
                    } else {
                        match element_kind.as_deref() {
                            // `Vec<String>` / `Vec<f32>` / ... - the item *is* the
                            // value, so any lookup name inside the list resolves to
                            // it. No `T: FieldAccess` bound needed.
                            Some(FieldKind::Text) => list_text.push(quote::quote! {
                                if *list_symbol == symbol_table::static_symbol!(#field_name)
                                    && let Some(item) = self.#field_ident.get(*index)
                                {
                                    return Some(item);
                                }
                            }),
                            Some(FieldKind::Bool) => list_bool.push(quote::quote! {
                                if *list_symbol == symbol_table::static_symbol!(#field_name)
                                    && let Some(item) = self.#field_ident.get(*index)
                                {
                                    return Some(*item);
                                }
                            }),
                            Some(FieldKind::Numeric) => list_numeric.push(quote::quote! {
                                if *list_symbol == symbol_table::static_symbol!(#field_name)
                                    && let Some(item) = self.#field_ident.get(*index)
                                {
                                    return Some(*item as f32);
                                }
                            }),
                            Some(FieldKind::Color) => list_color.push(quote::quote! {
                                if *list_symbol == symbol_table::static_symbol!(#field_name)
                                    && let Some(item) = self.#field_ident.get(*index)
                                {
                                    return Some(item);
                                }
                            }),
                            Some(FieldKind::Image) => list_image.push(quote::quote! {
                                if *list_symbol == symbol_table::static_symbol!(#field_name)
                                    && let Some(item) = self.#field_ident.get(*index)
                                {
                                    return Some(item);
                                }
                            }),
                            // `Vec<SomeStruct>` - each item supplies its own fields
                            // through `T: FieldAccess` (normally `#[derive(FieldAccess)]`).
                            _ => {
                                list_bool.push(quote::quote! {
                                    if *list_symbol == symbol_table::static_symbol!(#field_name)
                                        && let Some(item) = self.#field_ident.get(*index)
                                        && let Some(value) = telera_app::FieldAccess::field_bool(item, name)
                                    {
                                        return Some(value);
                                    }
                                });
                                list_numeric.push(quote::quote! {
                                    if *list_symbol == symbol_table::static_symbol!(#field_name)
                                        && let Some(item) = self.#field_ident.get(*index)
                                        && let Some(value) = telera_app::FieldAccess::field_numeric(item, name)
                                    {
                                        return Some(value);
                                    }
                                });
                                list_text.push(quote::quote! {
                                    if *list_symbol == symbol_table::static_symbol!(#field_name)
                                        && let Some(item) = self.#field_ident.get(*index)
                                        && let Some(value) = telera_app::FieldAccess::field_text(item, name)
                                    {
                                        return Some(value);
                                    }
                                });
                                list_color.push(quote::quote! {
                                    if *list_symbol == symbol_table::static_symbol!(#field_name)
                                        && let Some(item) = self.#field_ident.get(*index)
                                        && let Some(value) = telera_app::FieldAccess::field_color(item, name)
                                    {
                                        return Some(value);
                                    }
                                });
                                list_image.push(quote::quote! {
                                    if *list_symbol == symbol_table::static_symbol!(#field_name)
                                        && let Some(item) = self.#field_ident.get(*index)
                                        && let Some(value) = telera_app::FieldAccess::field_image(item, name)
                                    {
                                        return Some(value);
                                    }
                                });
                            }
                        }
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
                #(#plain_bool)*
                if let Some((list_symbol, index)) = list_data {
                    #(#list_bool)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_numeric(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<f32> {
                #(#plain_numeric)*
                if let Some((list_symbol, index)) = list_data {
                    #(#list_numeric)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_text<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<&'render_pass String>
            where
                'application: 'render_pass,
            {
                #(#plain_text)*
                if let Some((list_symbol, index)) = list_data {
                    #(#list_text)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_color<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<&'render_pass telera_app::Color>
            where
                'application: 'render_pass,
            {
                #(#plain_color)*
                if let Some((list_symbol, index)) = list_data {
                    #(#list_color)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_image<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<&'render_pass telera_app::UIImageDescriptor>
            where
                'application: 'render_pass,
            {
                #(#plain_image)*
                if let Some((list_symbol, index)) = list_data {
                    #(#list_image)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_event<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<symbol_table::GlobalSymbol>
            where
                'application: 'render_pass,
            {
                if let Some((list_symbol, index)) = list_data {
                    #(#list_event)*
                }
                None
            }
            #[allow(unused_variables)]
            fn get_list_length(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<usize> {
                #(#plain_list_length)*
                None
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
    let mut plain_image = Vec::<proc_macro2::TokenStream>::new();

    if let syn::Data::Struct(data) = &ast.data {
        for field in &data.fields {
            let Some(field_ident) = &field.ident else {
                continue;
            };
            let field_name = field_ident.to_string();
            match classify_field_type(&field.ty) {
                Some(FieldKind::Bool) => plain_bool.push(quote::quote! {
                    if *name == symbol_table::static_symbol!(#field_name) { return Some(self.#field_ident); }
                }),
                Some(FieldKind::Numeric) => plain_numeric.push(quote::quote! {
                    if *name == symbol_table::static_symbol!(#field_name) { return Some(self.#field_ident as f32); }
                }),
                Some(FieldKind::Text) => plain_text.push(quote::quote! {
                    if *name == symbol_table::static_symbol!(#field_name) { return Some(&self.#field_ident); }
                }),
                Some(FieldKind::Color) => plain_color.push(quote::quote! {
                    if *name == symbol_table::static_symbol!(#field_name) { return Some(&self.#field_ident); }
                }),
                Some(FieldKind::Image) => plain_image.push(quote::quote! {
                    if *name == symbol_table::static_symbol!(#field_name) { return Some(&self.#field_ident); }
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
                #(#plain_bool)*
                None
            }
            #[allow(unused_variables)]
            fn field_numeric(&self, name: &symbol_table::GlobalSymbol) -> Option<f32> {
                #(#plain_numeric)*
                None
            }
            #[allow(unused_variables)]
            fn field_text(&self, name: &symbol_table::GlobalSymbol) -> Option<&String> {
                #(#plain_text)*
                None
            }
            #[allow(unused_variables)]
            fn field_color(&self, name: &symbol_table::GlobalSymbol) -> Option<&telera_app::Color> {
                #(#plain_color)*
                None
            }
            #[allow(unused_variables)]
            fn field_image(&self, name: &symbol_table::GlobalSymbol) -> Option<&telera_app::UIImageDescriptor> {
                #(#plain_image)*
                None
            }
        }
    }
    .into()
}

/// `#[telera_app]` goes on an application's inherent `impl` block and writes
/// its [`LayoutReflector`] impl for it, wiring each method tagged with a
/// marker attribute into the right dispatcher:
///
/// * `#[layout_event]` - `fn name(&mut self, context: Option<EventContext>,
///   api: &mut API)` - reachable from a layout config like
///   `left-clicked name`, dispatched through `dispatch_event`.
/// * `#[layout_element]` - `fn name(&mut self, api: &mut API)` - reachable
///   from a `fn *name*` element, dispatched through
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
                #name_str => self.#name(api),
            });
        }
    }

    quote::quote! {
        #item_impl

        impl telera_app::LayoutReflector for #self_ty {
            #[allow(unused_variables)]
            fn dispatch_event(
                &mut self,
                name: &symbol_table::GlobalSymbol,
                context: ::core::option::Option<telera_app::EventContext>,
                api: &mut telera_app::API,
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
                api: &mut telera_app::API,
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

/// `#[layout_fn]` goes on a hand-written [`App::layout`] method and injects the
/// two layout-building macros into its body, so an app using the imperative
/// (non-markdown) layout path doesn't have to define them itself:
///
/// * `e!(config $(, child_stmt)* $(,)?)` - opens an element, configures it with
///   `&config`, runs each `child_stmt` (nested `e!` / `t!` calls, `if`s, loops,
///   ...) as its children, then closes it.
/// * `t!(text_config, content)` - adds `content` to the current element as a
///   text run styled by `text_config`.
///
/// Both expand to calls on a binding named `api` (the `api: &mut API`
/// parameter), so the parameter must keep that name.
///
/// ```ignore
/// impl App for MyApp {
///     #[layout_fn]
///     fn layout(&mut self, page: &str, api: &mut API) {
///         let row = ElementConfiguration::default().grow_all().end();
///         let label = TextConfig::new().font_size(16).end();
///         e!(row, t!(label, "hello"));
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn layout_fn(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut func = syn::parse_macro_input!(item as syn::ImplItemFn);
    let stmts = &func.block.stmts;

    func.block = syn::parse_quote!({
        #[allow(unused_macros)]
        macro_rules! e {
            ($v:expr $(, $c:stmt)* $(,)? ) => {
                api.l.open_element();
                api.l.configure_element(&$v);
                $(
                    $c
                )*
                api.l.close_element();
            };
        }

        #[allow(unused_macros)]
        macro_rules! t {
            ($v:expr, $c:expr) => {
                api.l.add_text_element($c, &$v, true);
            };
        }

        #(#stmts)*
    });

    quote::quote! { #func }.into()
}

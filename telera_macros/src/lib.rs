use syn::{GenericArgument, PathArguments, Type};


fn impl_handler_trait(abstract_syntax_tree: syn::DeriveInput) -> proc_macro::TokenStream {
    let enum_name = abstract_syntax_tree.ident.clone();
    let enum_span = abstract_syntax_tree.ident.span();

    let handler_for = abstract_syntax_tree.attrs.iter().filter(|attribute|{
        attribute.path().segments.len() == 1 &&
        attribute.path().is_ident("handler_for")
    }).nth(0).expect("no handler specified").clone();

    let user_application = if  let Ok(args) = handler_for.parse_args() &&
        let syn::Expr::Path(expression_path) = args &&
        let Some(user_application) = expression_path.path.get_ident()
    {
        user_application.clone()
    }
    else {
        panic!("input to \"handler_for\" must be struct type")
    };

    let variants = if let syn::Data::Enum(enum_data) = abstract_syntax_tree.data {

        let mut variants = Vec::<proc_macro2::TokenStream>::new();
        for enum_variant in enum_data.variants {
            let variant_name = enum_variant.ident.to_string();

            let re = regex::Regex::new(r"(\B)([A-Z])").expect("invalid regex");
            let mut handler_function_name = re.replace_all(&variant_name, "_$2").to_lowercase();

            handler_function_name.push_str("_handler");

            let handler_function = proc_macro2::Ident::new(&handler_function_name, enum_span);

            if variant_name.as_str() != "None" {
                let variant_name = proc_macro2::Ident::new(&variant_name, enum_span);

                variants.push(quote::quote! {
                    #enum_name::#variant_name => #handler_function(app,context,api),
                })
            }
        }
        variants
    } else {
        panic!("#[derive(Handler)] can only be used on enums");
    };

    quote::quote! {
        impl EventHandler for #enum_name {
            type UserApplication = #user_application;
            fn dispatch(&self, app: &mut Self::UserApplication, context: Option<EventContext>, api: &mut API) {
                match self {
                    #(#variants)*//#enum_name::Yes => yes_handler(app, api),
                    _ => {}
                }
            }
        }
    }.into()
}

#[proc_macro_derive(EventHandler, attributes(handler_for))]
pub fn handler_dispatch(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast: syn::DeriveInput = syn::parse(item).unwrap();
    impl_handler_trait(ast)
}

#[proc_macro_derive(ParserDataAccess, attributes(event_handler))]
pub fn parser_data_acces(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast: syn::DeriveInput = syn::parse(item).unwrap();
    let struct_name = ast.ident.clone();

    let event_handler = ast.attrs.iter().filter(|attribute|{
        attribute.path().segments.len() == 1 &&
        attribute.path().is_ident("event_handler")
    }).nth(0).expect("no handler specified").clone();

    let event_handler = if  let Ok(args) = event_handler.parse_args() &&
        let syn::Expr::Path(expression_path) = args &&
        let Some(user_application) = expression_path.path.get_ident()
    {
        user_application.clone()
    }
    else {
        panic!("input to \"event_handler\" must be an enum")
    };

    let mut numeric = Vec::<proc_macro2::TokenStream>::new();
    let mut boolean = Vec::<proc_macro2::TokenStream>::new();
    let mut _text = Vec::<proc_macro2::TokenStream>::new();
    let mut lists = Vec::<proc_macro2::TokenStream>::new();

    if let syn::Data::Struct(data) = ast.data {
        for field in data.fields {
            if let Some(field_ident) = field.ident
            && let syn::Type::Path(p) = field.ty 
            && let None = p.path.leading_colon
            && let Some(pp) = p.path.segments.get(0) {
                let data_type = pp.ident.to_string();
                let field_name = field_ident.clone().to_string();
                match data_type.as_str() {
                    "u8" |
                    "u16" |
                    "u32" |
                    "i8" |
                    "i16" |
                    "i32" |
                    "f8" |
                    "f16" |
                    "f32" => {
                        numeric.push(quote::quote! {
                            s if s == symbol_table::static_symbol!(#field_name) => Some(self.#field_ident as f32),
                        });
                    }
                    "bool" => {
                        boolean.push(quote::quote! {
                            s if s == symbol_table::static_symbol!(#field_name) => Some(self.#field_ident),
                        });
                    }
                    "String" => {

                    }
                    "Vec" => {
                        if let PathArguments::AngleBracketed(args) = &pp.arguments
                        && let Some(args) = args.args.get(0)
                        && let GenericArgument::Type(t) = args
                        && let Type::Path(vp) = t
                        && let None = vp.path.leading_colon
                        && let Some(_st) = vp.path.segments.get(0) {
                            //panic!("{:#?}", st.ident);
                            //let list_ident = st.ident.clone();
                            lists.push(quote::quote! {
                                s if s == symbol_table::static_symbol!(#field_name) => Some(self.#field_ident.len()),
                            });
                        }
                    }
                    _ => {}
                }
            }
            
        }
    }

    quote::quote! {
        impl ParserDataAccess<#event_handler> for #struct_name {
            fn get_bool(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<bool>{
                match *name {
                    #(#boolean)*
                    _ => None
                }
            }
            fn get_numeric(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<f32>{
                match *name {
                    #(#numeric)*
                    _ => None
                }
            }
            fn get_text<'render_pass, 'application>(&'application self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<&'render_pass String> where 'application: 'render_pass{
                match *name {
                    _ => None
                }
            }
            fn get_list_length(&self, name: &symbol_table::GlobalSymbol, list_data: &Option<(symbol_table::GlobalSymbol, usize)>) -> Option<usize> {
                match *name {
                    #(#lists)*
                    _ => None
                }
            }
        }
    }.into()
}

#[proc_macro_derive(App)]
pub fn app(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast: syn::DeriveInput = syn::parse(item).unwrap();
    let struct_name = ast.ident.clone();

    quote::quote! {
        impl App for #struct_name {}
    }.into()
}
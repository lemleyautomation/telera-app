
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
        enum_data.variants.into_iter().map(|enum_variant| {

            let mut variant_name = enum_variant.ident.to_string();

            let re = regex::Regex::new(r"(\B)([A-Z])").expect("invalid regex");
            variant_name = re.replace_all(&variant_name, "_$2").to_lowercase();

            variant_name.push_str("_handler");

            let handler_function = proc_macro2::Ident::new(&variant_name, enum_span);

            quote::quote! {
                #enum_name::#enum_variant => #handler_function(app,api),
            }
        }).collect::<Vec<proc_macro2::TokenStream>>()
    } else {
        panic!("#[derive(Handler)] can only be used on enums");
    };

    quote::quote! {
        impl EventHandler for #enum_name {
            type UserApplication = #user_application;
            fn dispatch(&self, app: &mut Self::UserApplication, api: &mut API) {
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
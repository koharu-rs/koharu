use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn specta(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

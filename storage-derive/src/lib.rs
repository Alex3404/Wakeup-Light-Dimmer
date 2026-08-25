use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input, Path, Expr};

use darling::FromDeriveInput;

#[derive(FromDeriveInput)]
#[darling(attributes(storage))]
struct StorageOpts {
    key_type: Path,
    key: Expr,
}

#[proc_macro_derive(StorageData, attributes(storage))]
pub fn derive_storage_data(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let opts = match StorageOpts::from_derive_input(&input) {
        Ok(o) => o,
        Err(e) => return e.write_errors().into(),
    };

    let key_type = opts.key_type;
    let key_value = opts.key;

    let expanded = quote! {
        impl<'a> #impl_generics crate::persistance::StorageData<'a, #key_type,
            { (core::mem::size_of::<Self>() + core::mem::size_of::<#key_type>()).next_multiple_of(4) + 120 }
        > for #name #ty_generics #where_clause {
            const BUFF_SIZE: usize = { (core::mem::size_of::<Self>() + core::mem::size_of::<#key_type>()).next_multiple_of(4) + 120 };
            const KEY: #key_type = #key_value;
        }

        impl<'a> sequential_storage::map::PostcardValue<'a> for #name {}
    };

    TokenStream::from(expanded)
}

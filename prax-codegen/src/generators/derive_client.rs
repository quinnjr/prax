//! Emit a `Client<E>` inside the per-model module with one accessor per
//! `prax_query::operations::*` builder. Each accessor clones the engine
//! stored in the Client and hands it to the operation builder.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub fn emit(model_name: &Ident) -> TokenStream {
    let client_ident = format_ident!("Client");
    quote! {
        pub struct #client_ident<E: ::prax_query::traits::QueryEngine> {
            engine: E,
        }

        impl<E: ::prax_query::traits::QueryEngine> #client_ident<E> {
            pub fn new(engine: E) -> Self { Self { engine } }

            pub fn find_many(&self)
                -> ::prax_query::operations::FindManyOperation<E, super::#model_name>
            { ::prax_query::operations::FindManyOperation::new(self.engine.clone()) }

            pub fn find_unique(&self)
                -> ::prax_query::operations::FindUniqueOperation<E, super::#model_name>
            { ::prax_query::operations::FindUniqueOperation::new(self.engine.clone()) }

            pub fn find_first(&self)
                -> ::prax_query::operations::FindFirstOperation<E, super::#model_name>
            { ::prax_query::operations::FindFirstOperation::new(self.engine.clone()) }

            pub fn create(&self)
                -> ::prax_query::operations::CreateOperation<E, super::#model_name>
            { ::prax_query::operations::CreateOperation::new(self.engine.clone()) }

            pub fn create_many(&self)
                -> ::prax_query::operations::CreateManyOperation<E, super::#model_name>
            { ::prax_query::operations::CreateManyOperation::new(self.engine.clone()) }

            pub fn update(&self)
                -> ::prax_query::operations::UpdateOperation<E, super::#model_name>
            { ::prax_query::operations::UpdateOperation::new(self.engine.clone()) }

            pub fn update_many(&self)
                -> ::prax_query::operations::UpdateManyOperation<E, super::#model_name>
            { ::prax_query::operations::UpdateManyOperation::new(self.engine.clone()) }

            pub fn upsert(&self)
                -> ::prax_query::operations::UpsertOperation<E, super::#model_name>
            { ::prax_query::operations::UpsertOperation::new(self.engine.clone()) }

            pub fn delete(&self)
                -> ::prax_query::operations::DeleteOperation<E, super::#model_name>
            { ::prax_query::operations::DeleteOperation::new(self.engine.clone()) }

            pub fn delete_many(&self)
                -> ::prax_query::operations::DeleteManyOperation<E, super::#model_name>
            { ::prax_query::operations::DeleteManyOperation::new(self.engine.clone()) }

            pub fn count(&self)
                -> ::prax_query::operations::CountOperation<E, super::#model_name>
            { ::prax_query::operations::CountOperation::new(self.engine.clone()) }
        }
    }
}

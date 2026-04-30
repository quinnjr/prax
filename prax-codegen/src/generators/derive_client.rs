//! Emit a `Client<E>` inside the per-model module with one accessor per
//! `prax_query::operations::*` builder. Each accessor clones the engine
//! stored in the Client and hands it to the operation builder.
//!
//! The `model_path` argument lets callers control how the model type is
//! referenced from inside the generated module. The `#[derive(Model)]`
//! path emits the struct at the parent scope, so passes `super::Foo`.
//! The `prax_schema!` path defines the struct inside the same module, so
//! passes `Foo`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub fn emit(model_path: TokenStream) -> TokenStream {
    let client_ident = format_ident!("Client");
    quote! {
        pub struct #client_ident<E: ::prax_query::traits::QueryEngine> {
            engine: E,
        }

        impl<E: ::prax_query::traits::QueryEngine> #client_ident<E> {
            pub fn new(engine: E) -> Self { Self { engine } }

            pub fn find_many(&self)
                -> ::prax_query::operations::FindManyOperation<E, #model_path>
            { ::prax_query::operations::FindManyOperation::new(self.engine.clone()) }

            pub fn find_unique(&self)
                -> ::prax_query::operations::FindUniqueOperation<E, #model_path>
            { ::prax_query::operations::FindUniqueOperation::new(self.engine.clone()) }

            pub fn find_first(&self)
                -> ::prax_query::operations::FindFirstOperation<E, #model_path>
            { ::prax_query::operations::FindFirstOperation::new(self.engine.clone()) }

            pub fn create(&self)
                -> ::prax_query::operations::CreateOperation<E, #model_path>
            { ::prax_query::operations::CreateOperation::new(self.engine.clone()) }

            pub fn create_many(&self)
                -> ::prax_query::operations::CreateManyOperation<E, #model_path>
            { ::prax_query::operations::CreateManyOperation::new(self.engine.clone()) }

            pub fn update(&self)
                -> ::prax_query::operations::UpdateOperation<E, #model_path>
            { ::prax_query::operations::UpdateOperation::new(self.engine.clone()) }

            pub fn update_many(&self)
                -> ::prax_query::operations::UpdateManyOperation<E, #model_path>
            { ::prax_query::operations::UpdateManyOperation::new(self.engine.clone()) }

            pub fn upsert(&self)
                -> ::prax_query::operations::UpsertOperation<E, #model_path>
            { ::prax_query::operations::UpsertOperation::new(self.engine.clone()) }

            pub fn delete(&self)
                -> ::prax_query::operations::DeleteOperation<E, #model_path>
            { ::prax_query::operations::DeleteOperation::new(self.engine.clone()) }

            pub fn delete_many(&self)
                -> ::prax_query::operations::DeleteManyOperation<E, #model_path>
            { ::prax_query::operations::DeleteManyOperation::new(self.engine.clone()) }

            pub fn count(&self)
                -> ::prax_query::operations::CountOperation<E, #model_path>
            { ::prax_query::operations::CountOperation::new(self.engine.clone()) }

            pub fn aggregate(&self)
                -> ::prax_query::operations::AggregateOperation<#model_path, E>
            { ::prax_query::operations::AggregateOperation::with_engine(self.engine.clone()) }

            pub fn group_by(&self, columns: Vec<String>)
                -> ::prax_query::operations::GroupByOperation<#model_path, E>
            { ::prax_query::operations::GroupByOperation::with_engine(self.engine.clone(), columns) }
        }
    }
}

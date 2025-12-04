#![allow(dead_code, unused_imports, non_camel_case_types)]
#![allow(clippy::all)]
#![allow(unused)]

pub mod errors;
mod generated;
mod impls;



pub use self::{
    errors::ModuleError,
    generated::{
        calls::{self, CallInfo},
        exports::*,
        runtime_types::runtime_types::{
            self, sp_runtime::DispatchError, vara_runtime, vara_runtime::RuntimeEvent as Event,
        },
        storage::{self, StorageInfo},
    },
    impls::Convert,
};
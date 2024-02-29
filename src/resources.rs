use std::{any::TypeId, collections::HashMap};

use crate::erased_data_vec::ErasedVec;

pub struct Resource<const SEND: bool> {
    data_storage: ErasedVec,
}

pub struct Resources<const SEND: bool> {
    resources: HashMap<TypeId, Resource<SEND>>,
}

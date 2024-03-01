use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    thread::ThreadId,
};

use crate::{
    erased_data_vec::{ErasedVec, UnsafeMutPtr, UnsafePtr},
    sparse_set::SparseSet,
    ComponentId, World,
};

pub(crate) trait ResourceBase {}

/// # Safety
///   The implementer must guarantee that SEND is true if and only if the resource is Send
pub unsafe trait Resource: 'static {
    const SEND: bool;
}

pub struct ResourceData<const SEND: bool> {
    data_storage: ErasedVec,
    type_name: String,

    // None for SEND resources
    original_creator: Option<ThreadId>,
}

pub struct Res<'world, 'res, T: 'static>
where
    'world: 'res,
{
    _ph: PhantomData<&'res T>,
    _ph_world: PhantomData<&'world World>,
    ptr: UnsafePtr<'res, T>,
    send: bool,
}

pub struct ResMut<'world, 'res, T: 'static>
where
    'world: 'res,
{
    _ph: PhantomData<&'res T>,
    _ph_world: PhantomData<&'world World>,
    ptr: UnsafeMutPtr<'res, T>,
}

impl<const SEND: bool> ResourceData<SEND> {
    fn new<R: 'static>(resource: R) -> Self {
        let mut vec = unsafe { ErasedVec::new_typed::<R>(true, 1) };
        unsafe { vec.push_back(resource) };
        Self {
            data_storage: vec,
            type_name: std::any::type_name::<R>().to_string(),
            original_creator: if SEND {
                None
            } else {
                Some(std::thread::current().id())
            },
        }
    }
    fn validate_access(&self) {
        if !SEND
            && self
                .original_creator
                .is_some_and(|id| id != std::thread::current().id())
        {
            panic!("Tried to read/write a non SEND resource from a thread that does not own it. Panicking");
        }
    }
}

#[derive(Default)]
pub(crate) struct Resources<const SEND: bool> {
    resources: SparseSet<ComponentId, ResourceData<SEND>>,
}

impl<const SEND: bool> Resources<SEND> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add<R: 'static>(&mut self, id: ComponentId, resource: R) {
        if let Some(old_resource) = self.resources.get_mut(id) {
            old_resource.validate_access();
            // SAFETY: The resource is present in the SparseSet
            // We also know that the type is correct because of the id
            unsafe { old_resource.data_storage.drop_at(0) };
            unsafe { old_resource.data_storage.insert_at(0, resource) };
        } else {
            let container = ResourceData::<SEND>::new(resource);
            self.resources.insert(id, container);
        }
    }

    // # Safety
    // The caller will ensure that, when accessing the pointer, no other mutable access is being performed
    pub unsafe fn get_ptr<R: 'static>(&self, id: ComponentId) -> Option<UnsafePtr<'_, R>> {
        self.resources.get(&id).map(|resource| {
            resource.validate_access();
            unsafe { resource.data_storage.get_ptr(0).cast::<R>() }
        })
    }

    // # Safety
    // The caller will ensure that, when accessing the pointer, no other access is being performed
    pub unsafe fn get_mut_ptr<R: 'static>(&self, id: ComponentId) -> Option<UnsafeMutPtr<'_, R>> {
        self.resources.get(&id).map(|resource| {
            resource.validate_access();
            unsafe { resource.data_storage.get_ptr(0).cast_mut::<R>() }
        })
    }

    // # Safety
    // The caller will ensure that, when accessing the pointer, no other mutable access is being performed
    pub(crate) unsafe fn get_unsafe_ref<R: 'static>(
        &self,
        id: ComponentId,
    ) -> Option<Res<'_, '_, R>> {
        self.get_ptr(id).map(|p| Res {
            _ph: PhantomData,
            _ph_world: PhantomData,
            ptr: p,
            send: SEND,
        })
    }

    // # Safety
    // The caller will ensure that, when accessing the pointer, no other access is being performed
    pub(crate) unsafe fn get_unsafe_mut_ref<R: 'static>(
        &self,
        id: ComponentId,
    ) -> Option<ResMut<'_, '_, R>> {
        self.get_mut_ptr(id).map(|p| ResMut {
            _ph: PhantomData,
            _ph_world: PhantomData,
            ptr: p,
        })
    }
}

impl<'world, 'res, T> Deref for Res<'world, 'res, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The caller must ensure that no mutable references are existing for the referred resource.
        unsafe { self.ptr.get() }
    }
}
impl<'world, 'res, T> Deref for ResMut<'world, 'res, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The caller must ensure that no mutable references are existing for the referred resource.
        unsafe { self.ptr.get() }
    }
}

impl<'world, 'res, T> DerefMut for ResMut<'world, 'res, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The caller must ensure that no mutable references are existing for the referred resource.
        unsafe { self.ptr.get_mut() }
    }
}

// SAFETY: The underlying resources are only accessed through & and &mut references
unsafe impl<'world, 'res, T> Send for Res<'world, 'res, T> {}
unsafe impl<'world, 'res, T> Send for ResMut<'world, 'res, T> {}
unsafe impl<'world, 'res, T> Sync for Res<'world, 'res, T> {}
unsafe impl<'world, 'res, T> Sync for ResMut<'world, 'res, T> {}

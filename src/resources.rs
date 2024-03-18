use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    thread::ThreadId,
};

use crate::{
    commands::TypedBlob,
    erased_data_vec::{ErasedVec, UnsafeMutPtr, UnsafePtr},
    sparse_set::SparseSet,
    ComponentId, WorldContainer,
};

/// This is a marker trait used to identify the structs that can be used as a Resource.
/// A Resource is a (singleton-like) object that can be accessed by systems using the
/// [`Res`] (for non-mutable, shared access)/[`ResMut`] (for mutable, single access) system parameters.
/// If the Resource is Non-Send, any access will be performed on the main thread
pub trait Resource: Send + Sync + 'static {}

pub struct ResourceData<const SEND: bool> {
    data_storage: ErasedVec,
    type_name: String,

    // None for SEND resources
    original_creator: Option<ThreadId>,
}

/// Provides non-mutable access to a resource stored in the [`crate::WorldContainer`]
/// To access a non-send resource non-mutably, use `&WorldContainer`
pub struct Res<'world, 'res, T: 'static>
where
    'world: 'res,
{
    pub(crate) _ph: PhantomData<&'res T>,
    pub(crate) _ph_world: PhantomData<&'world WorldContainer>,
    pub(crate) ptr: UnsafePtr<'res, T>,
}

/// Provides mutable access to a resource stored in the [`crate::WorldContainer`]
/// To access a non-send resource mutably, use `&mut WorldContainer`
pub struct ResMut<'world, 'res, T: 'static>
where
    'world: 'res,
{
    pub(crate) _ph: PhantomData<&'res T>,
    pub(crate) _ph_world: PhantomData<&'world WorldContainer>,
    pub(crate) ptr: UnsafeMutPtr<'res, T>,
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
    fn new_from_existing(storage: ErasedVec, type_name: &'static str) -> Self {
        assert!(storage.len() == 1);
        Self {
            data_storage: storage,
            type_name: type_name.to_string(),
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
            panic!("Tried to read/write non SEND resource '{}' from a thread that does not own it. Panicking", self.type_name);
        }
    }
}

#[derive(Default)]
pub(crate) struct Resources<const SEND: bool> {
    pub(crate) resources: SparseSet<ComponentId, ResourceData<SEND>>,
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

    /// # Safety
    /// The caller must ensure that id's type id corresponds to the type id of the resource
    pub unsafe fn add_dynamic(&mut self, id: ComponentId, resource: TypedBlob) {
        if let Some(old_resource) = self.resources.get_mut(id) {
            old_resource.validate_access();
            // SAFETY: The resource is present in the SparseSet
            // We also know that the type is correct because of the id
            unsafe { old_resource.data_storage.drop_at(0) };
            old_resource.data_storage.copy_from(0, &resource.data, 0);
        } else {
            let container = ResourceData::<SEND>::new_from_existing(
                resource.data,
                resource.type_name.expect("No type name"),
            );
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
    ) -> Option<UnsafePtr<'_, R>> {
        self.get_ptr(id)
    }

    // # Safety
    // The caller will ensure that, when accessing the pointer, no other access is being performed
    pub(crate) unsafe fn get_unsafe_mut_ref<R: 'static>(
        &self,
        id: ComponentId,
    ) -> Option<UnsafeMutPtr<'_, R>> {
        self.get_mut_ptr(id)
    }
}

impl<const SEND: bool> Drop for Resources<SEND> {
    fn drop(&mut self) {
        for res in self.resources.iter() {
            unsafe { res.1.data_storage.drop_at(0) };
        }
    }
}

impl<'world, 'res, T: Resource> Deref for Res<'world, 'res, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The caller must ensure that no mutable references are existing for the referred resource.
        unsafe { self.ptr.get() }
    }
}
impl<'world, 'res, T: Resource> Deref for ResMut<'world, 'res, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The caller must ensure that no mutable references are existing for the referred resource.
        unsafe { self.ptr.get() }
    }
}

impl<'world, 'res, T: Resource> DerefMut for ResMut<'world, 'res, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The caller must ensure that no mutable references are existing for the referred resource.
        unsafe { self.ptr.get_mut() }
    }
}

// SAFETY: The underlying resources are only accessed through & and &mut references
unsafe impl<'world, 'res, T: Resource> Send for Res<'world, 'res, T> {}
unsafe impl<'world, 'res, T: Resource> Send for ResMut<'world, 'res, T> {}
unsafe impl<'world, 'res, T: Resource> Sync for Res<'world, 'res, T> {}
unsafe impl<'world, 'res, T: Resource> Sync for ResMut<'world, 'res, T> {}

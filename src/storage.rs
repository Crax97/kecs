use std::marker::PhantomData;

use crate::{
    erased_data_vec::{ErasedVec, UnsafeMutPtr, UnsafePtr},
    sparse_set::SparseSet,
    ComponentId, Entity,
};

pub trait StorageType: Send + Sync + 'static {
    /// # Safety
    ///   The caller must ensure that the entity is not present in the storage
    unsafe fn register_new_entity(&mut self, entity: Entity);

    /// # Safety
    ///   The caller must ensure that the entity is present in the storage
    unsafe fn erase_entity(&mut self, entity: Entity);

    /// # Safety
    ///   1. The caller must ensure that T corresponds to the type of ComponentId
    ///   2. The caller must ensure that the specified entity does not have the specified component of type T
    unsafe fn add_entity_component<T: 'static>(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        component: T,
    );

    /// # Safety
    ///   1. The caller must ensure that there is at least one element inside the `data` ErasedVec
    ///   2. The data inside the first element of the ErasedVec must be of the same type of the component uniquely identified
    ///      by `component_id`
    ///   3. The caller must ensure that the specified entity does not have the specified component of the specified type
    unsafe fn add_entity_component_dynamic(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        data: &ErasedVec,
    );

    /// # Safety
    ///   1. The caller must ensure that the specified entity has the specified component
    ///   2. The caller must ensure that there is no other access to the component
    unsafe fn erase_entity_component(&mut self, entity: Entity, component_id: ComponentId);

    /// # Safety
    ///   1. The caller must ensure that the specified entity has the specified component
    ///   2. The caller must ensure that there is no other access to the component
    unsafe fn replace_entity_component<T: 'static>(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        component: T,
    ) {
        self.erase_entity_component(entity, component_id);
        self.add_entity_component(entity, component_id, component);
    }

    /// # Safety
    ///   1. The caller must ensure that the specified entity has the specified component
    ///   2. The caller must ensure that there is no other access to the component
    unsafe fn replace_entity_component_dynamic(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        data: &ErasedVec,
    ) {
        self.erase_entity_component(entity, component_id);
        self.add_entity_component_dynamic(entity, component_id, data)
    }

    /// # Safety
    ///   1. The caller must ensure that T corresponds to the type of ComponentId
    ///   2. The caller must ensure that the specified entity has the specified component of type T
    ///   3. The caller must ensure to not write the component while it's being read  
    ///   4. The caller must ensure that, while this or any pointer is alive, no changes to the underlying data structures
    ///      used by the storage type must be done
    unsafe fn get_component<T: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafePtr<T>;

    /// # Safety
    ///   1. The caller must ensure that T corresponds to the type of ComponentId
    ///   2. The caller must ensure that the specified entity has the specified component of type T
    ///   3. The caller must ensure that at any time, this is the only access that is reading or writing the component
    ///   4. The caller must ensure that, while any this or any pointer is alive, no changes to the underlying data structures
    ///      used by the storage type must be done
    unsafe fn get_component_mut<T: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafeMutPtr<T>;
}

pub struct TableStorage {
    columns: SparseSet<ComponentId, ErasedVec>,
    num_entities: usize,
}

impl TableStorage {
    pub fn new() -> Self {
        Self {
            columns: Default::default(),
            num_entities: 0,
        }
    }
}

impl StorageType for TableStorage {
    unsafe fn register_new_entity(&mut self, _entity: Entity) {
        self.num_entities += 1;
        for column in self.columns.iter_mut() {
            column.ensure_len(self.num_entities);
        }
    }

    unsafe fn erase_entity(&mut self, _entity: Entity) {
        // Nothing
    }

    unsafe fn add_entity_component<T: 'static>(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        component: T,
    ) {
        let component_storage = self.columns.get_or_insert(component_id, || unsafe {
            let mut vec = ErasedVec::new_typed::<T>(true, self.num_entities);
            vec.ensure_len(self.num_entities);
            vec
        });
        component_storage.insert_at(entity.0 as usize, component);
    }

    unsafe fn add_entity_component_dynamic(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        data: &ErasedVec,
    ) {
        let component_storage = self.columns.get_or_insert(component_id, || unsafe {
            let mut vec = ErasedVec::new(data.layout, data.drop_fn, 1);
            vec.ensure_len(self.num_entities);
            vec
        });
        component_storage.copy_from(entity.0 as usize, data, 0);
    }

    unsafe fn erase_entity_component(&mut self, entity: Entity, component_id: ComponentId) {
        let component_storage = self.columns.get_mut(component_id).unwrap();
        unsafe { component_storage.drop_at(entity.0 as usize) };
    }

    unsafe fn get_component<T: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafePtr<T> {
        let component_storage = self.columns.get(&component_id).unwrap();
        unsafe {
            let ptr = component_storage.get_ptr(entity.0 as usize).cast::<T>().0;
            UnsafePtr(ptr, PhantomData)
        }
    }

    /// # SAFETY
    ///   1. The caller must ensure that T corresponds to the type of ComponentId
    ///   2. The caller must ensure to not alias the component
    unsafe fn get_component_mut<T: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafeMutPtr<T> {
        let component_storage = self.columns.get(&component_id).unwrap();
        unsafe {
            let ptr = component_storage
                .get_ptr(entity.0 as usize)
                .cast_mut::<T>()
                .0;
            UnsafeMutPtr(ptr, PhantomData)
        }
    }
}

unsafe impl Send for TableStorage {}
unsafe impl Sync for TableStorage {}

use std::marker::PhantomData;

use crate::{
    archetype::{ArchetypeId, ArchetypeManager},
    erased_data_vec::{UnsafeMutPtr, UnsafePtr},
    sparse_set::SparseSet,
    storage::{StorageType, TableStorage},
    type_registrar::{TypeRegistrar, UniqueTypeId},
};

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Default, Clone, Copy)]
pub struct ComponentId(UniqueTypeId);

impl From<usize> for ComponentId {
    fn from(value: usize) -> Self {
        ComponentId(UniqueTypeId(value))
    }
}

impl From<ComponentId> for usize {
    fn from(value: ComponentId) -> Self {
        value.0 .0
    }
}

#[derive(Default, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct Entity(pub(crate) u32);

impl From<usize> for Entity {
    fn from(value: usize) -> Self {
        Entity(value as u32)
    }
}

impl From<Entity> for usize {
    fn from(value: Entity) -> Self {
        value.0 as usize
    }
}

#[derive(Default, Clone, Debug)]
pub(crate) struct EntityInfo {
    pub(crate) components: SparseSet<ComponentId, ()>,
    pub(crate) archetype_id: ArchetypeId,
}

pub struct Store {
    storage: TableStorage,
    next_entity_id: u32,
    entity_info: SparseSet<Entity, EntityInfo>,
    dropped_entities: Vec<Entity>,
    registrar: TypeRegistrar,
    archetype_manager: ArchetypeManager,
}

pub struct UnsafeStorePtr<'a>(UnsafeMutPtr<'a, Store>);
impl<'a> Clone for UnsafeStorePtr<'a> {
    fn clone(&self) -> Self {
        Self(UnsafeMutPtr(self.0 .0, PhantomData))
    }
}

impl Store {
    pub fn new() -> Self {
        Self {
            next_entity_id: 0,
            storage: TableStorage::new(),
            dropped_entities: Default::default(),
            entity_info: SparseSet::default(),
            registrar: TypeRegistrar::default(),
            archetype_manager: ArchetypeManager::default(),
        }
    }

    pub unsafe fn get_mut_ptr(&self) -> UnsafeStorePtr<'_> {
        UnsafeStorePtr(UnsafeMutPtr((self as *const Store).cast_mut(), PhantomData))
    }

    pub fn new_entity(&mut self) -> Entity {
        let id = self.next_entity_id;
        let id = Entity(id);
        self.next_entity_id += 1;
        self.entity_info.insert(id, EntityInfo::default());
        // SAFETY: The registered entity is a new entity
        unsafe {
            self.storage.register_new_entity(id);
        }
        id
    }

    pub fn remove_entity(&mut self, entity: Entity) {
        if let Some(info) = self.entity_info.get_mut(entity) {
            let components = info.components.iter().map(|(c, _)| c).collect::<Vec<_>>();
            for component in components {
                Self::remove_component_untyped(entity, info, component, &mut self.storage);
            }
            // SAFETY: An entity is alive only when it has an associated EntityInfo
            unsafe {
                self.storage.erase_entity(entity);
                self.entity_info.remove(entity);
            }
            self.dropped_entities.push(entity);
        }
    }

    pub fn iter_all_entities(&self) -> impl Iterator<Item = (Entity, &EntityInfo)> + '_ {
        self.entity_info.iter()
    }

    pub fn add_component<C: 'static>(&mut self, entity: Entity, component: C) {
        let component_id = ComponentId(self.registrar.get_registration::<C>());
        let entity_info = self.entity_info.get_mut(entity).unwrap();
        if entity_info.components.contains(&component_id) {
            //# SAFETY: The entity contains the specified component
            unsafe {
                self.storage
                    .replace_entity_component(entity, component_id, component);
            };
            return;
        } else {
            entity_info.components.insert(component_id, ());
        }

        self.update_entity_archetype(entity);

        //# SAFETY: The entity does not have the specified component
        unsafe {
            self.storage
                .add_entity_component(entity, component_id, component);
        }
    }

    pub fn get_archetype_manager_mut(&mut self) -> &mut ArchetypeManager {
        &mut self.archetype_manager
    }

    pub fn get_archetype_manager(&self) -> &ArchetypeManager {
        &self.archetype_manager
    }

    fn update_entity_archetype(&mut self, entity: Entity) {
        let entity_info = self.entity_info.get_mut(entity).unwrap();
        if let Some(old_archetype) = self
            .archetype_manager
            .get_archetype_mut(entity_info.archetype_id)
        {
            old_archetype.entities.remove(entity);
        }

        let new_archetype_id = self.archetype_manager.archetype_of(&entity_info.components);
        entity_info.archetype_id = new_archetype_id;

        let new_archetype = self
            .archetype_manager
            .get_archetype_mut(entity_info.archetype_id)
            .unwrap();
        new_archetype.entities.insert(entity, ());
    }

    pub(crate) unsafe fn get_component<C: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafePtr<'_, C> {
        let entity_info = &self.entity_info.get(&entity).unwrap().components;
        assert!(entity_info.contains(&component_id));
        //# SAFETY: We asserted that the entity has the component
        unsafe { self.storage.get_component(entity, component_id) }
    }
    pub(crate) unsafe fn get_component_mut<C: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafeMutPtr<'_, C> {
        let entity_info = &self.entity_info.get(&entity).unwrap().components;
        assert!(entity_info.contains(&component_id));
        //# SAFETY: We asserted that the entity has the component
        unsafe { self.storage.get_component_mut(entity, component_id) }
    }

    pub fn remove_component<C: 'static>(&mut self, entity: Entity) {
        let component_id = ComponentId(self.registrar.get_registration::<C>());
        if let Some(entity_info) = self.entity_info.get_mut(entity) {
            Self::remove_component_untyped(entity, entity_info, component_id, &mut self.storage);
            self.update_entity_archetype(entity);
        }
    }

    fn remove_component_untyped(
        entity: Entity,
        entity_info: &mut EntityInfo,
        component_id: ComponentId,
        storage: &mut TableStorage,
    ) {
        let entity_components = &mut entity_info.components;
        if entity_components.contains(&component_id) {
            //# SAFETY: We know for sure that the entity has the specified component
            unsafe { storage.erase_entity_component(entity, component_id) };
            entity_components.remove(component_id);
        }
    }
    pub(crate) fn entity_info(&self, entity: Entity) -> Option<&EntityInfo> {
        self.entity_info.get(&entity)
    }

    pub fn get_component_id_mut<A: 'static>(&mut self) -> ComponentId {
        ComponentId(self.registrar.get_registration::<A>())
    }
    pub fn get_component_id<A: 'static>(&self) -> ComponentId {
        ComponentId(self.registrar.get::<A>())
    }
    pub fn get_component_id_maybe<A: 'static>(&self) -> Option<ComponentId> {
        self.registrar.get_maybe::<A>().map(ComponentId)
    }

    pub fn entity_has_component<A: 'static>(&self, entity: Entity) -> bool {
        self.get_component_id_maybe::<A>().is_some_and(|id| {
            self.entity_info(entity)
                .is_some_and(|e| e.components.contains(&id))
        })
    }

    pub fn get_entity_info(&self, e: Entity) -> Option<&EntityInfo> {
        self.entity_info.get(&e)
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let entities = self.entity_info.iter().map(|(e, _)| e).collect::<Vec<_>>();
        for ent in entities {
            self.remove_entity(ent);
        }
    }
}

impl<'a> UnsafeStorePtr<'a> {
    pub unsafe fn get(&self) -> &Store {
        self.0.get()
    }
    pub unsafe fn get_mut(&mut self) -> &mut Store {
        self.0.get_mut()
    }
    pub unsafe fn get_component<A: 'static>(&self, entity: Entity) -> UnsafePtr<'a, A> {
        let store = unsafe { self.0 .0.as_mut().unwrap() };
        let component_id = store.get_component_id::<A>();
        store.get_component(entity, component_id)
    }

    pub unsafe fn get_component_mut<A: 'static>(&self, entity: Entity) -> UnsafeMutPtr<'a, A> {
        let store = unsafe { self.0 .0.as_mut().unwrap() };
        let component_id = store.get_component_id::<A>();
        store.get_component_mut(entity, component_id)
    }
}

unsafe impl<'a> Send for UnsafeStorePtr<'a> {}
unsafe impl<'a> Sync for UnsafeStorePtr<'a> {}

// pub unsafe trait SystemParamArg {
//     type Arg;
//     fn component_ids(store: &mut Store) -> Vec<ComponentId>;
//     fn extract(indices: &[usize], vecs: &[&UnsafeCell<ErasedVec>], base: &mut usize) -> Self;
// }

// unsafe impl<A: 'static> SystemParamArg for &A {
//     type Arg = A;
//     fn component_ids(store: &mut Store) -> Vec<ComponentId> {
//         vec![store.get_component_id::<A>()]
//     }
//     fn extract(indices: &[usize], vecs: &[&UnsafeCell<ErasedVec>], base: &mut usize) -> Self {
//         let idx = *base;
//         *base += 1;
//         let vec = &vecs[idx];
//         let vec = unsafe { vec.get().as_mut().unwrap() };
//         unsafe { std::mem::transmute(vec.get::<Self::Arg>(indices[idx])) }
//     }
// }

// unsafe impl<A: 'static> SystemParamArg for &mut A {
//     type Arg = A;
//     fn component_ids(store: &mut Store) -> Vec<ComponentId> {
//         vec![store.get_component_id::<A>()]
//     }
//     fn extract(indices: &[usize], vecs: &[&UnsafeCell<ErasedVec>], base: &mut usize) -> Self {
//         let idx = *base;
//         *base += 1;
//         let vec = &vecs[idx];
//         let vec = unsafe { vec.get().as_mut().unwrap() };
//         unsafe { std::mem::transmute(vec.get_mut::<Self::Arg>(indices[idx])) }
//     }
// }

// unsafe impl<A: 'static, B: 'static> SystemParamArg for (A, B)
// where
//     A: SystemParamArg,
//     B: SystemParamArg,
// {
//     type Arg = (A, B);
//     fn component_ids(store: &mut Store) -> Vec<ComponentId> {
//         A::component_ids(store)
//             .into_iter()
//             .chain(B::component_ids(store))
//             .collect()
//     }

//     fn extract(indices: &[usize], vecs: &[&UnsafeCell<ErasedVec>], base: &mut usize) -> Self {
//         (
//             A::extract(indices, vecs, base),
//             B::extract(indices, vecs, base),
//         )
//     }
// }

// unsafe impl<A: 'static, B: 'static, C: 'static> SystemParamArg for (A, B, C)
// where
//     A: SystemParamArg,
//     B: SystemParamArg,
//     C: SystemParamArg,
// {
//     type Arg = (A, B, C);
//     fn component_ids(store: &mut Store) -> Vec<ComponentId> {
//         A::component_ids(store)
//             .into_iter()
//             .chain(B::component_ids(store))
//             .chain(C::component_ids(store))
//             .collect()
//     }

//     fn extract(indices: &[usize], vecs: &[&UnsafeCell<ErasedVec>], base: &mut usize) -> Self {
//         (
//             A::extract(indices, vecs, base),
//             B::extract(indices, vecs, base),
//             C::extract(indices, vecs, base),
//         )
//     }
// }

// pub trait SystemParam {
//     fn create(store: &mut Store) -> Self
//     where
//         Self: 'static;
// }

// pub trait System<T> {
//     fn exec(&mut self, store: &mut Store);
// }

// pub struct Query<A: SystemParamArg> {
//     _ph_data: PhantomData<A>,
//     components: Vec<ComponentId>,
//     store: *mut Store,
//     entities: Vec<EntityInfo>,
//     current_idx: usize,
// }

// impl<A: SystemParamArg> SystemParam for Query<A> {
//     fn create(store: &mut Store) -> Self
//     where
//         Self: 'static,
//     {
//         Self {
//             _ph_data: PhantomData,
//             components: A::component_ids(store),
//             current_idx: 0,
//             store: store as *mut Store,
//             entities: store.entity_info.iter().cloned().collect(),
//         }
//     }
// }

// impl<A: SystemParamArg> Query<A> {
//     pub fn advance(&mut self) -> Option<A> {
//         println!("Component ids {:?}", self.components);
//         let store = unsafe { self.store.as_mut().unwrap() };
//         let vecs = store.extract_vecs(&self.components).unwrap();

//         if self.current_idx == self.entities.len() {
//             return None;
//         }
//         while let Some(entity) = self.entities.get(self.current_idx) {
//             self.current_idx += 1;
//             println!("Checking entity with components {:?}", entity.components);
//             if let Some(indices) = entity.components.get_multi(&self.components) {
//                 let mut idx = 0;
//                 let a = A::extract(&indices, &vecs, &mut idx);

//                 return Some(a);
//             }
//         }

//         return None;
//     }
// }

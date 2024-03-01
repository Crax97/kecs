use std::marker::PhantomData;

use crate::{
    archetype::{ArchetypeId, ArchetypeManager},
    erased_data_vec::{UnsafeMutPtr, UnsafePtr},
    resources::{Resource, Resources},
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

pub struct World {
    storage: TableStorage,
    next_entity_id: u32,
    entity_info: SparseSet<Entity, EntityInfo>,
    dropped_entities: Vec<Entity>,
    registrar: TypeRegistrar,
    archetype_manager: ArchetypeManager,

    pub(crate) send_resources: Resources<true>,
    pub(crate) non_send_resources: Resources<false>,
    // This SparseSet contains true if the resource is Send, false otherwise
    pub(crate) resource_sendness: SparseSet<ComponentId, bool>,
}

pub struct UnsafeWorldPtr<'a>(UnsafeMutPtr<'a, World>);
impl<'a> Clone for UnsafeWorldPtr<'a> {
    fn clone(&self) -> Self {
        Self(UnsafeMutPtr(self.0 .0, PhantomData))
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            next_entity_id: 0,
            storage: TableStorage::new(),
            dropped_entities: Default::default(),
            entity_info: SparseSet::default(),
            registrar: TypeRegistrar::default(),
            archetype_manager: ArchetypeManager::default(),
            send_resources: Resources::new(),
            non_send_resources: Resources::new(),
            resource_sendness: Default::default(),
        }
    }

    pub unsafe fn get_mut_ptr(&self) -> UnsafeWorldPtr<'_> {
        UnsafeWorldPtr(UnsafeMutPtr((self as *const World).cast_mut(), PhantomData))
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

    pub fn add_resource<R: 'static + Send + Sync + Resource>(&mut self, resource: R) {
        let id = self.get_component_id_mut::<R>();
        self.resource_sendness.insert(id, true);
        self.send_resources.add(id, resource);
    }

    pub fn add_non_send_resource<R: 'static + Resource>(&mut self, resource: R) {
        let id = self.get_component_id_mut::<R>();
        self.resource_sendness.insert(id, false);
        self.non_send_resources.add(id, resource);
    }

    fn get_resource<R: Resource + 'static>(&self) -> Option<&R> {
        self.get_component_id::<R>()
            // SAFETY: This is safe because we're accessing a &R through a &World
            .and_then(|id| unsafe {
                let is_send = *self.resource_sendness.get(&id).unwrap();
                if is_send {
                    self.send_resources.get_ptr(id)
                } else {
                    self.non_send_resources.get_ptr(id)
                }
            })
            .map(|p| unsafe { std::mem::transmute::<&R, &R>(p.get()) })
    }

    fn get_resource_mut<R: Resource + 'static>(&mut self) -> Option<&mut R> {
        self.get_component_id::<R>()
            // SAFETY: This is safe because we're accessing a &mut R through a &mut World
            .and_then(|id| unsafe {
                let is_send = *self.resource_sendness.get(&id).unwrap();
                if is_send {
                    self.send_resources.get_mut_ptr(id)
                } else {
                    self.non_send_resources.get_mut_ptr(id)
                }
            })
            .map(|mut p| unsafe { std::mem::transmute::<&mut R, &mut R>(p.get_mut()) })
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
    pub fn get_component_id<A: 'static>(&self) -> Option<ComponentId> {
        self.registrar.get_maybe::<A>().map(ComponentId)
    }

    // Panics if the type wasn't registered (e.g by adding it beforehand)
    pub fn get_component_id_assertive<A: 'static>(&self) -> ComponentId {
        ComponentId(self.registrar.get::<A>())
    }

    pub fn entity_has_component<A: 'static>(&self, entity: Entity) -> bool {
        self.get_component_id::<A>().is_some_and(|id| {
            self.entity_info(entity)
                .is_some_and(|e| e.components.contains(&id))
        })
    }

    pub fn get_entity_info(&self, e: Entity) -> Option<&EntityInfo> {
        self.entity_info.get(&e)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let entities = self.entity_info.iter().map(|(e, _)| e).collect::<Vec<_>>();
        for ent in entities {
            self.remove_entity(ent);
        }
    }
}

impl<'a> UnsafeWorldPtr<'a> {
    pub unsafe fn get(&self) -> &World {
        self.0.get()
    }
    pub unsafe fn get_mut(&mut self) -> &mut World {
        self.0.get_mut()
    }
    pub unsafe fn get_component<A: 'static>(&self, entity: Entity) -> UnsafePtr<'a, A> {
        let store = unsafe { self.0 .0.as_mut().unwrap() };
        let component_id = store.get_component_id_assertive::<A>();
        store.get_component(entity, component_id)
    }

    pub unsafe fn get_component_mut<A: 'static>(&self, entity: Entity) -> UnsafeMutPtr<'a, A> {
        let store = unsafe { self.0 .0.as_mut().unwrap() };
        let component_id = store.get_component_id_assertive::<A>();
        store.get_component_mut(entity, component_id)
    }
}

unsafe impl<'a> Send for UnsafeWorldPtr<'a> {}
unsafe impl<'a> Sync for UnsafeWorldPtr<'a> {}

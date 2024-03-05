use std::{any::TypeId, marker::PhantomData};

use crate::{
    archetype::ArchetypeManager,
    commands::{Commands, CommandsSender, TypedBlob},
    entity_manager::EntityAllocator,
    erased_data_vec::{ErasedVec, UnsafeMutPtr, UnsafePtr},
    resources::{Resource, Resources},
    sparse_set::SparseSet,
    storage::{StorageType, TableStorage},
    type_registrar::{TypeRegistrar, UniqueTypeId},
    Entity, EntityInfo,
};

/// The unique id of any component
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

/// A [`WorldContainer`] holds all the state for the [`Entity`] in the [`crate::KecsWorld`], their components
/// and the World's [`Resource`]s.
/// The [`WorldContainer`] can be either modified from the [`crate::KecsWorld`], or can be modified by a system using
/// using a parameter of type `&mut `[`WorldContainer`]: such systems can only have the [`WorldContainer`] as their parameter,
/// and are called *exclusive systems*, and will be scheduled to be run on the main thread
///
/// e.g
///```
/// use kecs::WorldContainer;
/// fn do_something(world: &mut WorldContainer) {
///    // Do something with the world
/// }
///```
pub struct WorldContainer {
    storage: TableStorage,
    registrar: TypeRegistrar,

    pub(crate) entity_manager: EntityAllocator,
    pub(crate) archetype_manager: ArchetypeManager,
    pub(crate) commands: CommandsSender,
    pub(crate) send_resources: Resources<true>,
    pub(crate) non_send_resources: Resources<false>,
    // This SparseSet contains true if the resource is Send, false otherwise
    pub(crate) resource_sendness: SparseSet<ComponentId, bool>,
}

// Functions exposed to systems
impl WorldContainer {
    /// Creates a [`Commands`] instance that can be used to send deferred commands
    pub fn commands(&self) -> Commands {
        Commands::new(self)
    }

    /// Gets a reference to a component for an [`Entity`], returns None if the component id does not exists
    /// or if the entity does not have the component
    pub fn get_component<T: 'static>(&self, entity: Entity) -> Option<&T> {
        let id = self.get_component_id::<T>()?;
        let has_component = self
            .get_entity_info(entity)
            .is_some_and(|info| info.components.contains(&id));
        if has_component {
            unsafe {
                // SAFETY: We chechked that the entity has the component, and that the component ID exists
                let ptr = self.storage.get_component(entity, id);
                Some(ptr.into_ref())
            }
        } else {
            None
        }
    }

    /// Gets a mutable reference to a component for an [`Entity`], returns None if the component id does not exists
    /// or if the entity does not have the component
    pub fn get_component_mut<T: 'static>(&mut self, entity: Entity) -> Option<&mut T> {
        let id = self.get_component_id::<T>()?;
        let has_component = self
            .get_entity_info(entity)
            .is_some_and(|info| info.components.contains(&id));
        if has_component {
            unsafe {
                // SAFETY: We chechked that the entity has the component, and that the component ID exists
                let ptr = self.storage.get_component_mut(entity, id);
                Some(ptr.into_mut())
            }
        } else {
            None
        }
    }

    /// Creates a new `Send` resource: a resource can be accessed by a system either through
    /// [`crate::Res`] or [`crate::ResMut`], access to the resource will be done in parallel when possible
    pub fn add_resource<R: 'static + Send + Sync + Resource>(&mut self, resource: R) {
        let id = self.get_or_create_component_id::<R>();
        self.resource_sendness.insert(id, true);
        self.send_resources.add(id, resource);
    }

    /// Creates a new `!Send` resource: accessing this resource can only be done on the main thread
    pub fn add_non_send_resource<R: 'static + Resource>(&mut self, resource: R) {
        let id = self.get_or_create_component_id::<R>();
        self.resource_sendness.insert(id, false);
        self.non_send_resources.add(id, resource);
    }

    /// Gets a reference to a resource, if it exists
    pub fn get_resource<R: Resource + 'static>(&self) -> Option<&R> {
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

    /// Gets a mutable reference to a resource, if it exists
    pub fn get_resource_mut<R: Resource + 'static>(&mut self) -> Option<&mut R> {
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

    /// Iterates all the [`Entity`]s, along with their [`EntityInfo`]s
    pub fn iter_all_entities(&self) -> impl Iterator<Item = (Entity, &EntityInfo)> + '_ {
        self.entity_manager.iter_all_entities()
    }

    /// Gets the [`EntityInfo`] associated to an entity
    pub fn get_entity_info(&self, e: Entity) -> Option<&EntityInfo> {
        self.entity_manager.entity_info(e)
    }

    /// Gets the [`ComponentId`] for type A if it exists, or creates a new one
    pub fn get_or_create_component_id<A: 'static>(&mut self) -> ComponentId {
        ComponentId(self.registrar.get_registration::<A>())
    }

    /// Gets the [`ComponentId`] for type A if it exists
    pub fn get_component_id<A: 'static>(&self) -> Option<ComponentId> {
        self.registrar.get_maybe::<A>().map(ComponentId)
    }

    /// Gets the [`ComponentId] for A, panicking if it doesn't exists
    pub fn get_component_id_assertive<A: 'static>(&self) -> ComponentId {
        ComponentId(self.registrar.get::<A>())
    }

    /// Returns `true` if the [`Entity`] has a component of type `A`
    pub fn entity_has_component<A: 'static>(&self, entity: Entity) -> bool {
        self.get_component_id::<A>().is_some_and(|id| {
            self.get_entity_info(entity)
                .is_some_and(|e| e.components.contains(&id))
        })
    }

    pub(crate) fn get_archetype_manager_mut(&mut self) -> &mut ArchetypeManager {
        &mut self.archetype_manager
    }

    pub(crate) fn get_archetype_manager(&self) -> &ArchetypeManager {
        &self.archetype_manager
    }

    pub(crate) unsafe fn add_component_from_type_id(
        &mut self,
        entity: Entity,
        component: TypedBlob,
    ) {
        let component_id = self.get_or_create_component_id_dynamic(component.blob_ty_id);
        self.add_component_dynamic(entity, component_id, &component.data);
    }

    fn get_or_create_component_id_dynamic(&mut self, blob_ty_id: TypeId) -> ComponentId {
        let id = self.registrar.get_from_type_id(blob_ty_id);
        ComponentId(id)
    }

    fn add_component_dynamic(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        data: &ErasedVec,
    ) {
        let entity_info = self
            .entity_manager
            .entity_info_mut(entity)
            .expect("Failed to get entity");
        if entity_info.components.contains(&component_id) {
            //# SAFETY: The entity contains the specified component
            unsafe {
                self.storage
                    .replace_entity_component_dynamic(entity, component_id, data);
            };
            return;
        }

        entity_info.components.insert(component_id, ());

        //# SAFETY: The entity does not have the specified component
        unsafe {
            self.storage
                .add_entity_component_dynamic(entity, component_id, data)
        }

        self.update_entity_archetype(entity);
    }

    pub(crate) fn remove_component_from_type_id(&mut self, entity: Entity, component_ty: TypeId) {
        let component_id = self.get_or_create_component_id_dynamic(component_ty);
        Self::remove_component_untyped(
            entity,
            self.entity_manager.entity_info_mut(entity).unwrap(),
            component_id,
            &mut self.storage,
        );
        self.update_entity_archetype(entity);
    }
}

/// An unsafe pointer to a [`WorldContainer`]
pub struct UnsafeWorldPtr<'a>(UnsafeMutPtr<'a, WorldContainer>);
impl<'a> Clone for UnsafeWorldPtr<'a> {
    fn clone(&self) -> Self {
        Self(UnsafeMutPtr(self.0 .0, PhantomData))
    }
}

impl WorldContainer {
    pub(crate) fn new(commands: CommandsSender) -> Self {
        Self {
            storage: TableStorage::new(),
            entity_manager: Default::default(),
            registrar: TypeRegistrar::default(),
            archetype_manager: ArchetypeManager::default(),
            send_resources: Resources::new(),
            non_send_resources: Resources::new(),
            resource_sendness: Default::default(),
            commands,
        }
    }

    pub(crate) unsafe fn get_mut_ptr(&mut self) -> UnsafeWorldPtr<'_> {
        UnsafeWorldPtr(UnsafeMutPtr(self as *mut _, PhantomData))
    }

    pub(crate) fn new_entity(&mut self) -> Entity {
        let id = self.entity_manager.new_entity();
        // SAFETY: The registered entity is a new entity
        unsafe {
            self.storage.register_new_entity(id);
        }
        id
    }

    /// # Safety
    /// The caller must ensure that the entity id is new
    pub(crate) unsafe fn new_entity_with_id(&mut self, id: Entity) {
        self.entity_manager.new_with_id(id);

        // SAFETY: The registered entity is a new entity
        self.storage.register_new_entity(id);
    }

    pub(crate) fn remove_entity(&mut self, entity: Entity) {
        if let Some(info) = self.entity_manager.entity_info_mut(entity) {
            let components = info.components.iter().map(|(c, _)| c).collect::<Vec<_>>();
            for component in components {
                Self::remove_component_untyped(entity, info, component, &mut self.storage);
            }
            // SAFETY: An entity is alive only when it has an associated EntityInfo
            unsafe {
                self.storage.erase_entity(entity);
            }
            self.entity_manager.destroy_entity(entity);
        }
    }

    pub(crate) fn add_component<C: 'static>(&mut self, entity: Entity, component: C) {
        let component_id = ComponentId(self.registrar.get_registration::<C>());
        let entity_info = self
            .entity_manager
            .entity_info_mut(entity)
            .expect("Failed to find entity");
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

    fn update_entity_archetype(&mut self, entity: Entity) {
        let entity_info = self
            .entity_manager
            .entity_info_mut(entity)
            .expect("Failed to find entity");
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

    pub(crate) unsafe fn get_component_unsafe<C: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafePtr<'_, C> {
        let entity_info = &self.entity_manager.entity_info(entity).unwrap().components;
        assert!(entity_info.contains(&component_id));
        //# SAFETY: We asserted that the entity has the component
        unsafe { self.storage.get_component(entity, component_id) }
    }
    pub(crate) unsafe fn get_component_mut_unsafe<C: 'static>(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> UnsafeMutPtr<'_, C> {
        let entity_info = &self.entity_manager.entity_info(entity).unwrap().components;
        assert!(entity_info.contains(&component_id));
        //# SAFETY: We asserted that the entity has the component
        unsafe { self.storage.get_component_mut(entity, component_id) }
    }

    pub(crate) fn remove_component<C: 'static>(&mut self, entity: Entity) {
        let component_id = ComponentId(self.registrar.get_registration::<C>());
        if let Some(entity_info) = self.entity_manager.entity_info_mut(entity) {
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
}

impl Drop for WorldContainer {
    fn drop(&mut self) {
        let entities = self.iter_all_entities().map(|(e, _)| e).collect::<Vec<_>>();
        for ent in entities {
            self.remove_entity(ent);
        }
    }
}

impl<'a> UnsafeWorldPtr<'a> {
    pub(crate) unsafe fn get_component<A: 'static>(&self, entity: Entity) -> UnsafePtr<'a, A> {
        let store = unsafe { self.0 .0.as_mut().unwrap() };
        let component_id = store.get_component_id_assertive::<A>();
        store.get_component_unsafe(entity, component_id)
    }

    pub(crate) unsafe fn get_component_mut<A: 'static>(
        &self,
        entity: Entity,
    ) -> UnsafeMutPtr<'a, A> {
        let store = unsafe { self.0 .0.as_mut().unwrap() };
        let component_id = store.get_component_id_assertive::<A>();
        store.get_component_mut_unsafe(entity, component_id)
    }
}

unsafe impl<'a> Send for UnsafeWorldPtr<'a> {}
unsafe impl<'a> Sync for UnsafeWorldPtr<'a> {}

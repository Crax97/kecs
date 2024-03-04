use crate::{system::IntoSystem, Entity, GraphScheduler, Resource, Scheduler, WorldContainer};

/// The [`KecsWorld`] is a wrapper around a [`Scheduler`] and the [`WorldContainer`] it acts on
pub struct KecsWorld<S: Scheduler = GraphScheduler> {
    container: WorldContainer,
    scheduler: S,
}

impl<S: Scheduler> KecsWorld<S> {
    /// Creates a new [`KecsWorld`] with a scheduler of type `S`
    pub fn new() -> Self {
        Self {
            container: WorldContainer::new(),
            scheduler: S::default(),
        }
    }

    /// Creates a new entity
    pub fn new_entity(&mut self) -> Entity {
        self.container.new_entity()
    }

    /// Adds a component to the [`Entity`]: if the entity already had the component, it is overwritten
    pub fn add_component<T: 'static>(&mut self, entity: Entity, component: T) {
        self.container.add_component(entity, component);

        self.update_systems(entity);
    }

    /// Removes a Component from the [`Entity`], if it has one
    pub fn remove_component<T: 'static>(&mut self, entity: Entity) {
        self.container.remove_component::<T>(entity);

        self.update_systems(entity);
    }

    /// Gets a reference to the Component from the [`Entity`] if it has one
    pub fn get_component<T: 'static>(&self, entity: Entity) -> Option<&T> {
        self.container.get_component::<T>(entity)
    }

    /// Gets a mutable reference to the Component from the [`Entity`] if it has one
    pub fn get_component_mut<T: 'static>(&mut self, entity: Entity) -> Option<&mut T> {
        self.container.get_component_mut::<T>(entity)
    }

    /// Adds a new Send resource: if the resource already exists, it is overwritten
    pub fn add_resource<T: 'static + Resource + Send + Sync>(&mut self, resource: T) {
        self.container.add_resource::<T>(resource);
    }

    /// Adds a new Non-Send resource: if the resource already exists, it is overwritten
    pub fn add_non_send_resource<T: 'static + Resource>(&mut self, resource: T) {
        self.container.add_non_send_resource::<T>(resource);
    }

    /// Gets a reference to the resource, if it exists
    pub fn get_resource<T: 'static + Resource>(&self) -> Option<&T> {
        self.container.get_resource()
    }

    /// Gets a mutable reference to the resource, if it exists
    pub fn get_resource_mut<T: 'static + Resource>(&mut self) -> Option<&mut T> {
        self.container.get_resource_mut()
    }

    /// Adds a system to the world, that will then be scheduled according to the [`crate::Scheduler`]
    pub fn add_system<ARGS, SYS: IntoSystem<ARGS>>(&mut self, system: SYS) -> S::SystemId {
        self.scheduler.add_system(&mut self.container, system)
    }

    /// Runs all the scheduled [`crate::System`]
    pub fn update(&mut self) {
        self.scheduler.execute(&mut self.container);
    }
}

impl<S: Scheduler> Default for KecsWorld<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Scheduler> KecsWorld<S> {
    fn update_systems(&mut self, entity: Entity) {
        self.scheduler
            .on_entity_updated(&mut self.container, entity);
    }
}

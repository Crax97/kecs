use crate::{system::IntoSystem, Entity, GraphScheduler, Resource, Scheduler, WorldContainer};

pub struct KecsWorld<S: Scheduler = GraphScheduler> {
    container: WorldContainer,
    scheduler: S,
}

impl<S: Scheduler> KecsWorld<S> {
    pub fn new() -> Self {
        Self {
            container: WorldContainer::new(),
            scheduler: S::default(),
        }
    }

    pub fn new_entity(&mut self) -> Entity {
        self.container.new_entity()
    }

    pub fn add_component<T: 'static>(&mut self, entity: Entity, component: T) {
        self.container.add_component(entity, component);

        self.update_systems(entity);
    }

    pub fn remove_component<T: 'static>(&mut self, entity: Entity) {
        self.container.remove_component::<T>(entity);

        self.update_systems(entity);
    }

    pub fn add_resource<T: 'static + Resource + Send + Sync>(&mut self, resource: T) {
        self.container.add_resource::<T>(resource);
    }

    pub fn add_non_send_resource<T: 'static + Resource>(&mut self, resource: T) {
        self.container.add_non_send_resource::<T>(resource);
    }

    pub fn get_resource<T: 'static + Resource>(&self) -> Option<&T> {
        self.container.get_resource()
    }

    pub fn get_resource_mut<T: 'static + Resource>(&mut self) -> Option<&mut T> {
        self.container.get_resource_mut()
    }

    pub fn add_system<ARGS, SYS: IntoSystem<ARGS>>(&mut self, system: SYS) -> S::SystemId {
        self.scheduler.add_system(&mut self.container, system)
    }

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

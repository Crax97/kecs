use std::any::TypeId;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::commands::{CommandType, Commands, CommandsReceiver, TypedBlob};
use crate::{
    system::IntoSystem, Entity, GraphScheduler, Resource, Scheduler, System, WorldContainer,
};

/// The [`KecsWorld`] is a wrapper around a [`Scheduler`] and the [`WorldContainer`] it acts on
pub struct KecsWorld<S: Scheduler = GraphScheduler> {
    container: WorldContainer,
    schedulers: HashMap<Label, S>,

    commands_receiver: CommandsReceiver,
}

/// A [`Label`] is used to identify a set of systems that should run together in a [`KecsWorld`]
#[derive(Clone, Copy, Hash, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct Label(u64);

/// Anything that can be turn into a [`Label`]
/// This trait is automatically implemented for everything [`Hash`]able
pub trait IntoLabel {
    /// Turns self into a [`Label`]
    fn into_label(self) -> Label;
}

impl<S: Scheduler> KecsWorld<S> {
    /// Creates a new [`KecsWorld`] with a scheduler of type `S`
    pub fn new() -> Self {
        let (commands, commands_receiver) = Commands::create();
        Self {
            container: WorldContainer::new(commands),
            schedulers: Default::default(),
            commands_receiver,
        }
    }

    /// Creates a new entity
    pub fn new_entity(&mut self) -> Entity {
        self.container.new_entity()
    }

    /// Destroys an entity, along with all of its components
    pub fn destroy_entity(&mut self, entity: Entity) {
        self.container.remove_entity(entity);
        self.update_systems(entity);
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
    pub fn add_system<ARGS, SYS: IntoSystem<ARGS>>(
        &mut self,
        label: impl IntoLabel,
        system: SYS,
    ) -> S::SystemId {
        self.schedulers
            .entry(label.into_label())
            .or_default()
            .add_system(&mut self.container, system)
    }

    /// Runs a system exclusively
    pub fn run_oneshot<ARGS, SYS: IntoSystem<ARGS>>(&mut self, system: SYS) {
        let mut system = system.into_system();
        system.init(&mut self.container);

        self.container.iter_all_entities().for_each(|(e, info)| {
            system.on_entity_changed(&self.container, e, info);
        });

        system.run(&mut self.container);
    }

    /// Executes the queued [`Commands`] and runs all the scheduled [`crate::System`] within a [`Label`]
    pub fn update(&mut self, label: impl IntoLabel) {
        self.execute_commands();
        self.schedulers
            .entry(label.into_label())
            .or_default()
            .execute(&mut self.container);
    }

    /// Creates the [`Commands`] for this World
    pub fn commands(&self) -> Commands {
        self.container.commands()
    }

    /// Gets a reference to this world's [`Scheduler`]
    pub fn scheduler(&self, label: impl IntoLabel) -> &S {
        self.schedulers
            .get(&label.into_label())
            .expect("Failed to find systems with this label")
    }
}

impl<S: Scheduler> Drop for KecsWorld<S> {
    fn drop(&mut self) {
        // Execute pending commands to avoid possible memory leaks
        self.execute_commands();
    }
}

impl<S: Scheduler> KecsWorld<S> {
    fn execute_commands(&mut self) {
        while let Some(command_type) = self.commands_receiver.try_get() {
            match command_type {
                CommandType::NewEntity { entity, components } => {
                    self.spawn_new_entity(entity, components);
                }
                CommandType::AddComponent { entity, component } => {
                    self.add_component_dynamic(entity, component, true);
                }
                CommandType::RemoveComponent {
                    entity,
                    component_ty,
                    component_ty_name,
                } => {
                    self.remove_component_dynamic(entity, component_ty, component_ty_name);
                }
                CommandType::AddResource { resource } => {
                    self.add_resource_dynamic(resource);
                }
                CommandType::DestroyEntity { entity } => self.destroy_entity(entity),
            }
        }
    }

    fn spawn_new_entity(&mut self, entity: Entity, components: HashMap<TypeId, TypedBlob>) {
        // SAFETY: We got this entity id from a command, which allocated it through the EntityManager
        unsafe { self.container.new_entity_with_id(entity) }

        for (_, component) in components {
            self.add_component_dynamic(entity, component, false);
        }

        self.update_systems(entity);
    }

    fn add_component_dynamic(&mut self, entity: Entity, component: TypedBlob, update_entity: bool) {
        // SAFETY: The typed blob was created by directly taking the typed component
        unsafe { self.container.add_component_from_type_id(entity, component) };

        if update_entity {
            self.update_systems(entity);
        }
    }

    fn remove_component_dynamic(
        &mut self,
        entity: Entity,
        component_ty: TypeId,
        type_name: &'static str,
    ) {
        self.container
            .remove_component_from_type_id(entity, component_ty, type_name);

        self.update_systems(entity);
    }

    fn add_resource_dynamic(&mut self, _resource: TypedBlob) {
        todo!()
    }
}

impl<T: Hash> IntoLabel for T {
    fn into_label(self) -> Label {
        let mut hasher = std::hash::DefaultHasher::new();
        self.hash(&mut hasher);
        Label(hasher.finish())
    }
}

impl<S: Scheduler> Default for KecsWorld<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Scheduler> KecsWorld<S> {
    fn update_systems(&mut self, entity: Entity) {
        self.schedulers.values_mut().for_each(|s| {
            s.on_entity_updated(&mut self.container, entity);
        });
    }
}

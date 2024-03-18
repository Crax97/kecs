use std::{
    any::{type_name, TypeId},
    collections::HashMap,
};

use crossbeam::channel::{unbounded, Receiver, Sender};

use crate::{
    entity_manager::EntityAllocator, erased_data_vec::ErasedVec, Entity, Resource, WorldContainer,
};

/// [`Commands`] is a system parameter used to queue operations that change the state of the world, such as
/// spawning a new [`crate::Entity`] or adding a component to an entity. During an update loop (read: frame),
/// all the commands are queue and then executed at the beginning of the frame
pub struct Commands<'world> {
    sender: CommandsSender,
    entity_allocator: &'world EntityAllocator,
}

/// [`EntityBuilder`] is a helper struct used to define the properties of a new entity.
/// The new entity id will be active from the next update cycle
pub struct EntityBuilder<'c, 'world>
where
    'world: 'c,
{
    commands: &'c mut Commands<'world>,
    components: HashMap<TypeId, TypedBlob>,
}

impl<'world> Commands<'world> {
    pub(crate) fn new(world: &'world WorldContainer) -> Self {
        Self {
            sender: world.commands.clone(),
            entity_allocator: &world.entity_manager,
        }
    }

    /// Starts an [`EntityBuilder`] instance
    pub fn spawn_entity<'c>(&'c mut self) -> EntityBuilder<'c, 'world>
    where
        'world: 'c,
    {
        EntityBuilder {
            commands: self,
            components: Default::default(),
        }
    }

    /// Destroys the given entity if it exists
    pub fn destroy_entity(&mut self, entity: Entity) {
        self.sender
            .inner
            .send(CommandType::DestroyEntity { entity })
            .expect("Failed to send DestroyEntity command");
    }

    /// Adds a new component to an existing entity, replacing any old ones of the same type
    pub fn add_component<T: 'static>(&mut self, entity: Entity, component: T) {
        self.sender
            .inner
            .send(CommandType::AddComponent {
                entity,
                component: TypedBlob::new(component),
            })
            .expect("Failed to send AddCommand command");
    }

    /// Removes a component from an entity if it exists
    pub fn remove_component<T: 'static>(&mut self, entity: Entity) {
        self.sender
            .inner
            .send(CommandType::RemoveComponent {
                entity,
                component_ty: TypeId::of::<T>(),
                component_ty_name: type_name::<T>(),
            })
            .expect("Failed to send RemoveComponent command");
    }

    /// Adds a new resource, replacing the old value if it does not exists
    pub fn add_resource<R: 'static + Resource + Send>(&mut self, resource: R) {
        self.sender
            .inner
            .send(CommandType::AddResource {
                resource: TypedBlob::new(resource),
                send: true,
            })
            .expect("Failed to send AddResource command");
    }
    /// Adds a new resource, replacing the old value if it does not exists
    pub fn add_non_send_resource<R: 'static + Resource>(&mut self, resource: R) {
        self.sender
            .inner
            .send(CommandType::AddResource {
                resource: TypedBlob::new(resource),
                send: false,
            })
            .expect("Failed to send AddResource command");
    }
}

impl<'c, 'world> EntityBuilder<'c, 'world>
where
    'world: 'c,
{
    /// Adds a new component to the new entity
    pub fn with_component<T: 'static>(&mut self, component: T) -> &mut Self {
        let entry = self
            .components
            .insert(TypeId::of::<T>(), TypedBlob::new(component));
        if let Some(entry) = entry {
            unsafe {
                // We know that the component is of the correct type
                entry.data.drop_at(0);
            }
        }
        self
    }

    /// Sends the command and returns the new entity id
    pub fn build(self) -> Entity {
        let entity = self.commands.entity_allocator.allocate_id();
        self.commands
            .sender
            .inner
            .send(CommandType::NewEntity {
                entity,
                components: self.components,
            })
            .expect("Failed to send command");
        entity
    }
}

pub(crate) struct TypedBlob {
    pub(crate) blob_ty_id: TypeId,
    pub(crate) data: ErasedVec,
    pub(crate) type_name: Option<&'static str>,
}

impl TypedBlob {
    fn new<T: 'static>(data: T) -> Self {
        let vec = unsafe {
            let mut vec = ErasedVec::new_typed::<T>(true, 1);
            vec.push_back(data);
            vec
        };
        Self {
            blob_ty_id: TypeId::of::<T>(),
            data: vec,
            type_name: Some(type_name::<T>()),
        }
    }
}

pub(crate) enum CommandType {
    NewEntity {
        entity: Entity,
        components: HashMap<TypeId, TypedBlob>,
    },
    DestroyEntity {
        entity: Entity,
    },
    AddComponent {
        entity: Entity,
        component: TypedBlob,
    },
    RemoveComponent {
        entity: Entity,
        component_ty: TypeId,
        component_ty_name: &'static str,
    },
    AddResource {
        resource: TypedBlob,
        send: bool,
    },
}

pub(crate) struct CommandsReceiver {
    receiver: Receiver<CommandType>,
}

#[derive(Clone)]
pub(crate) struct CommandsSender {
    inner: Sender<CommandType>,
}

impl<'world> Commands<'world> {
    pub(crate) fn create() -> (CommandsSender, CommandsReceiver) {
        let (sender, receiver) = unbounded();

        (
            CommandsSender { inner: sender },
            CommandsReceiver { receiver },
        )
    }
}

impl CommandsReceiver {
    pub(crate) fn try_get(&mut self) -> Option<CommandType> {
        self.receiver.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use crate::{commands::Commands, Entity, Query, Res, ResMut, Resource, World};

    fn make_world() -> World {
        World::new()
    }

    #[test]
    fn spawn_entity() {
        let mut world = make_world();

        struct TestCounter {
            counter: Arc<RwLock<usize>>,
        }

        struct CounterRes {
            counter: Arc<RwLock<usize>>,
        }
        impl Resource for CounterRes {}

        let counter_out = Arc::<RwLock<usize>>::default();

        let spawn_entity = |mut commands: Commands, counter: Res<CounterRes>| {
            let mut spawn = commands.spawn_entity();
            spawn.with_component(TestCounter {
                counter: counter.counter.clone(),
            });
            spawn.build();
        };

        fn increment_counter(query: Query<&TestCounter>) {
            query.iter().for_each(|counter| {
                let mut counter = counter.counter.write().unwrap();
                *counter += 1;
            })
        }

        world.add_resource(CounterRes {
            counter: counter_out,
        });
        world.add_system(0, spawn_entity);
        world.add_system(0, increment_counter);

        // No TestCounters spawned
        world.update(0);
        assert_eq!(
            *world
                .get_resource::<CounterRes>()
                .unwrap()
                .counter
                .read()
                .unwrap(),
            0
        );

        // // One TestCounter spawned
        world.update(0);
        assert_eq!(
            *world
                .get_resource::<CounterRes>()
                .unwrap()
                .counter
                .read()
                .unwrap(),
            1
        );

        // Two TestCounters spawned
        world.update(0);
        assert_eq!(
            *world
                .get_resource::<CounterRes>()
                .unwrap()
                .counter
                .read()
                .unwrap(),
            3
        );
    }

    #[test]
    fn destroy_entity() {
        let mut world = make_world();

        struct SpawnCount {
            count: u32,
        }
        impl Resource for SpawnCount {}

        struct Bullet {
            ticks_alive: u32,
        }

        let destroy_counter = Arc::<RwLock<usize>>::default();
        let destroy_counter_2 = destroy_counter.clone();
        world.add_resource(SpawnCount { count: 10 });
        world.add_system(
            0,
            |mut counter: ResMut<SpawnCount>, mut commands: Commands| {
                for _ in 0..counter.count {
                    let mut entity = commands.spawn_entity();
                    entity.with_component(Bullet { ticks_alive: 10 });
                    entity.build();
                }
                counter.count = 0;
            },
        );
        world.add_system(
            0,
            move |bullets: Query<(Entity, &mut Bullet)>, mut commands: Commands| {
                for (entity, bullet) in bullets.iter() {
                    if bullet.ticks_alive == 0 {
                        let mut destroyed = destroy_counter_2.write().unwrap();
                        *destroyed += 1;
                        commands.destroy_entity(entity);
                    } else {
                        bullet.ticks_alive -= 1;
                    }
                }
            },
        );

        for _ in 0..100 {
            world.update(0);
        }

        assert_eq!(*destroy_counter.read().unwrap(), 10);
    }

    #[test]
    fn remove_component() {
        let mut world = make_world();

        struct TestCounter {
            counter: Arc<RwLock<usize>>,
        }

        let counter_out = Arc::<RwLock<usize>>::default();
        let counter = counter_out.clone();
        let get_counter = || -> usize { *counter.read().unwrap() };

        let counter = counter_out.clone();
        let spawn_entity = move |mut commands: Commands| {
            let mut spawn = commands.spawn_entity();
            spawn.with_component(TestCounter {
                counter: counter.clone(),
            });
            let entity = spawn.build();

            commands.remove_component::<TestCounter>(entity);
        };

        fn increment_counter(mut commands: Commands, query: Query<(Entity, &TestCounter)>) {
            query.iter().for_each(|(entity, counter)| {
                let mut counter = counter.counter.write().unwrap();
                *counter += 1;

                commands.destroy_entity(entity);
            })
        }

        world.add_system(0, spawn_entity);
        world.add_system(0, increment_counter);

        // No TestCounters spawned
        world.update(0);
        assert_eq!(get_counter(), 0);

        // One TestCounter spawned, but it was destroyed
        world.update(0);
        assert_eq!(get_counter(), 0);

        // Two TestCounters spawned, but they were destroyed
        world.update(0);
        assert_eq!(get_counter(), 0);
    }
}

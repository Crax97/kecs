#![warn(missing_docs)]

//! The KECS component system is a simple [Entity-Component-System](https://en.wikipedia.org/wiki/Entity_component_system) implementation, that strives to be small.
//!
//! A [`crate::KecsWorld`] is an object composed of [`Entity`]s, associated to one or multiple components. These components can be interacted
//! with by adding [`crate::System`]s to the [`crate::KecsWorld`]
//! Here's a short example
//! ```
//! use kecs::{World, Query};
//!
//! // Components are plain Rust structs
//! struct Foo(u32);
//! struct Baz(u32);
//! struct Name(String);
//! // Create a world with the [`crate::GraphScheduler`], which allows for systems to be run in parallel
//! let mut world = World::new();
//!
//! let entity = world.new_entity();
//! world.add_component(entity, Foo(10));
//! world.add_component(entity, Name("John".to_string()));
//!
//! let entity = world.new_entity();
//! world.add_component(entity, Baz(10));
//! world.add_component(entity, Name("Frank".to_string()));
//!
//! // You can iterate on multipler components by using a tuple, e.g Query<(&mut MutatedComponent, &NonMutatedComponent)>
//! fn iter_only_foos(query: Query<&mut Foo>) {
//!     for item in query.iter() {
//!         item.0 = 123;
//!     }
//! }
//!
//! fn iter_only_bazs(query: Query<&mut Baz>) {
//!     for item in query.iter() {
//!         item.0 = 456;
//!     }
//! }
//!
//! fn print_names(query: Query<&mut Name>) {
//!     for item in query.iter() {
//!         println!("item name {}", item.0);
//!     }    
//! }
//! // To group systems together you can use a Label
//! let run_systems = "label";
//! world.add_system(run_systems, iter_only_foos);
//! world.add_system(run_systems, iter_only_bazs);
//! world.add_system(run_systems, print_names);
//!
//! // Fire in the hole!
//! world.update(run_systems);
//! ```
mod archetype;
mod entity_manager;
mod erased_data_vec;
mod query;
mod resources;
mod schedule;
mod storage;
mod system;
mod type_registrar;
mod world;
mod world_container;

mod commands;
mod sparse_set;

pub use archetype::*;
pub use commands::{Commands, EntityBuilder};
pub use entity_manager::{Entity, EntityInfo};
pub use query::*;
pub use resources::{Res, ResMut, Resource};
pub use schedule::{GraphScheduler, LinearScheduler, Scheduler};
pub use sparse_set::SparseSet;
pub use system::{System, SystemContainer, SystemParam};
pub use world::*;
pub use world_container::*;

/// The suggested [`KecsWorld`] to use: the [`GraphScheduler`] will run systems in parallel when possible
pub type World = KecsWorld<GraphScheduler>;

impl<'cworld> SystemParam for Commands<'cworld> {
    type State = ();

    const IS_WORLD: bool = false;

    fn add_dependencies(
        _store: &mut WorldContainer,
        _components: &mut SparseSet<ComponentId, AccessMode>,
    ) {
    }

    fn create<'world, 'state>(_data: &'state Self::State, store: &'world mut WorldContainer) -> Self
    where
        'world: 'state,
    {
        unsafe { std::mem::transmute(store.commands()) }
    }

    fn create_initial_state(_store: &mut WorldContainer) -> Self::State {}

    fn on_entity_changed(
        _state: &mut Self::State,
        _store: &WorldContainer,
        _entity: Entity,
        _info: &EntityInfo,
    ) {
    }

    fn is_exclusive(_world: &WorldContainer) -> bool {
        false
    }

    fn on_entity_destroyed(_state: &mut Self::State, _store: &WorldContainer, _entity: Entity) {}
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use crate::{query::Query, IntoLabel, World};

    #[test]
    fn iter_n_times() {
        struct TestComponent {
            counter: Arc<RwLock<usize>>,
        }

        fn counting_system(query: Query<&mut TestComponent>) {
            for counter in query.iter() {
                let mut lock = counter.counter.write().unwrap();
                *lock = lock.overflowing_add(1).0;
            }
        }

        let counter = Arc::<RwLock<usize>>::default();

        let mut world = World::new();

        const ENTITIES: usize = 100;
        for _ in 0..ENTITIES {
            let entity = world.new_entity();
            world.add_component(
                entity,
                TestComponent {
                    counter: counter.clone(),
                },
            );
        }

        let label = "test".into_label();
        world.add_system(label, counting_system);
        world.update(label);

        assert_eq!(counter.read().unwrap().to_owned(), ENTITIES);
    }

    #[test]
    fn add_after_schedule() {
        struct TestComponent {
            counter: Arc<RwLock<usize>>,
        }

        fn counting_system(query: Query<&mut TestComponent>) {
            for counter in query.iter() {
                let mut lock = counter.counter.write().unwrap();
                *lock = lock.overflowing_add(1).0;
            }
        }

        let counter = Arc::<RwLock<usize>>::default();

        let mut world = World::new();

        let label = "test".into_label();
        world.add_system(label, counting_system);

        const ENTITIES: usize = 100;
        for _ in 0..ENTITIES {
            let entity = world.new_entity();
            world.add_component(
                entity,
                TestComponent {
                    counter: counter.clone(),
                },
            );
        }

        world.update(label);

        assert_eq!(counter.read().unwrap().to_owned(), ENTITIES);
    }

    #[test]
    fn add_remove() {
        struct TestComponent {
            counter: Arc<RwLock<usize>>,
        }

        fn counting_system(query: Query<&mut TestComponent>) {
            for counter in query.iter() {
                let mut lock = counter.counter.write().unwrap();
                *lock = lock.overflowing_add(1).0;
            }
        }

        let counter = Arc::<RwLock<usize>>::default();

        let mut world = World::new();

        const ENTITIES: usize = 100;
        for _ in 0..ENTITIES {
            let entity = world.new_entity();
            world.add_component(
                entity,
                TestComponent {
                    counter: counter.clone(),
                },
            );
        }

        let entity = world.new_entity();
        world.add_component(
            entity,
            TestComponent {
                counter: counter.clone(),
            },
        );

        let label = "test".into_label();
        world.add_system(label, counting_system);

        world.remove_component::<TestComponent>(entity);
        world.update(label);

        assert_eq!(counter.read().unwrap().to_owned(), ENTITIES);
    }

    #[test]
    fn labels_update_all_systems() {
        let lab_1 = "lab_1".into_label();
        let lab_2 = "lab_2".into_label();

        let mut world = World::new();

        struct TestComponentA;

        let counter = Arc::<RwLock<usize>>::default();
        let counter_2 = counter.clone();
        let iter_all_components = move || {
            let counter = counter_2.clone();
            move |query: Query<&TestComponentA>| {
                for _ in query.iter() {
                    *counter.write().unwrap() += 1
                }
            }
        };

        world.add_system(lab_1, iter_all_components());
        world.add_system(lab_2, iter_all_components());

        world.update(lab_1);
        world.update(lab_2);
        assert_eq!(*counter.read().unwrap(), 0);

        let ent = world.new_entity();
        world.add_component(ent, TestComponentA);

        world.update(lab_1);
        world.update(lab_2);
        assert_eq!(*counter.read().unwrap(), 2);
    }
}

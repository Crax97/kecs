mod archetype;
mod erased_data_vec;
mod query;
mod resources;
mod schedule;
mod sparse_set;
mod storage;
mod system;
mod type_registrar;
mod world;
mod world_container;

pub use query::*;
pub use resources::{Res, Resource};
pub use schedule::{GraphScheduler, LinearScheduler, Scheduler};
pub use system::{System, SystemContainer, SystemParam};
pub use world::*;
pub use world_container::*;

pub type World = KecsWorld<GraphScheduler>;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use crate::{query::Query, World};

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

        world.add_system(counting_system);
        world.update();

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

        world.add_system(counting_system);

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

        world.update();

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

        world.add_system(counting_system);

        world.remove_component::<TestComponent>(entity);
        world.update();

        assert_eq!(counter.read().unwrap().to_owned(), ENTITIES);
    }
}

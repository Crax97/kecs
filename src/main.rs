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

use resources::{Res, Resource};
use system::{System, SystemContainer, SystemParam};
use world::*;

use crate::query::Query;

struct IntComponent {
    data: i32,
}

struct FloatComponent {
    data: f32,
}

struct StringComponent {
    data: String,
}

struct TestResource {
    cool_number: u32,
}

unsafe impl Resource for TestResource {
    const SEND: bool = true;
}

fn system_i(query: Query<&IntComponent>, resource: Res<TestResource>) {
    for component in query.iter() {
        println!("Component value is {}", component.data);
    }
}

fn system_f(comp: &mut FloatComponent) {
    println!("float is {}", comp.data);
    comp.data *= 100.0;
}

fn system_s(comp: &StringComponent) {
    println!("string is {}", comp.data);
}

fn system_if((comp, flo): (&IntComponent, &FloatComponent)) {
    println!("int is {} float is {}", comp.data, flo.data);
}

fn system_is((int, stri): (&IntComponent, &StringComponent)) {
    println!("int is {}, stri is {}", int.data, stri.data);
}

fn system_fs((flo, stri): (&FloatComponent, &StringComponent)) {
    println!("flo is {}, stri is {}", flo.data, stri.data);
}

fn system_ifs((int, stri, flo): (&IntComponent, &StringComponent, &FloatComponent)) {
    println!(
        "int is {}, stri is {}, flo is {}",
        int.data, stri.data, flo.data
    );
}

fn test_run_system<'param, A, B, F: Fn(A, B) + Send + Sync + 'static>(store: &mut World, fun: F)
where
    A: SystemParam + Send + Sync + 'static,
    B: SystemParam + Send + Sync + 'static,
{
    let mut system = SystemContainer::<F, (A, B)>::new(fun);
    system.init(store);
    system.run(store);
}

fn main() {
    let mut world = World::new();
    world.add_resource(TestResource { cool_number: 42 });
    let entity_0 = world.new_entity();
    world.add_component::<IntComponent>(entity_0, IntComponent { data: 10 });

    let entity_1 = world.new_entity();
    world.add_component::<FloatComponent>(
        entity_1,
        FloatComponent {
            data: std::f32::consts::PI,
        },
    );
    world.add_component::<IntComponent>(entity_1, IntComponent { data: 42 });

    let entity_2 = world.new_entity();
    world.add_component::<IntComponent>(entity_2, IntComponent { data: 1234 });
    world.add_component::<StringComponent>(
        entity_2,
        StringComponent {
            data: "Hello World".to_owned(),
        },
    );

    let entity_3 = world.new_entity();
    world.add_component::<IntComponent>(entity_3, IntComponent { data: 974 });
    world.add_component::<FloatComponent>(entity_3, FloatComponent { data: 0.15566 });
    world.add_component::<StringComponent>(
        entity_3,
        StringComponent {
            data: "Ciao Mondo!".to_owned(),
        },
    );

    // println!("Running one-param systems");
    test_run_system(&mut world, system_i);

    // let mut query = Query::<TableStorage, &mut FloatComponent>::create_query(&mut store);
    // query.exec(system_f);

    // let mut query = Query::<TableStorage, &StringComponent>::create_query(&mut store);
    // query.exec(system_s);

    // println!("Running two-param systems");
    // let mut query =
    //     Query::<TableStorage, (&IntComponent, &FloatComponent)>::create_query(&mut store);
    // query.exec(system_if);
    // let mut query =
    //     Query::<TableStorage, (&IntComponent, &StringComponent)>::create_query(&mut store);
    // query.exec(system_is);
    // let mut query =
    //     Query::<TableStorage, (&FloatComponent, &StringComponent)>::create_query(&mut store);
    // query.exec(system_fs);

    // println!("Running three-param systems");
    // let mut query =
    //     Query::<TableStorage, (&IntComponent, &StringComponent, &FloatComponent)>::create_query(
    //         &mut store,
    //     );
    // query.exec(system_ifs);
}

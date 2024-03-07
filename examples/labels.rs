use kecs::*;
use rand::random;

#[derive(Debug)]
pub struct Position([f32; 2]);

#[derive(Debug)]
pub struct Velocity([f32; 2]);
pub struct Player;

const NUM_ENTITIES: usize = 1000;
fn setup_entities(mut commands: Commands) {
    for _ in 0..NUM_ENTITIES {
        let mut builder = commands.spawn_entity();
        builder
            .with_component(Position([0.0; 2]))
            .with_component(Velocity([random::<f32>().fract(), random::<f32>().fract()]));
        builder.build();
    }

    let mut builder = commands.spawn_entity();
    builder
        .with_component(Position([0.0; 2]))
        .with_component(Velocity([random::<f32>().fract(), random::<f32>().fract()]))
        .with_component(Player);
    builder.build();
}

fn update_entities_position(query: Query<(&mut Position, &Velocity)>) {
    for (pos, vel) in query.iter() {
        pos.0[0] += vel.0[0];
        pos.0[1] += vel.0[1];
    }
}

fn main() {
    let mut world = World::new();

    world.add_system("startup", setup_entities);
    world.add_system("update", update_entities_position);

    world.update("startup");
    for _ in 0..100 {
        world.update("update");
    }
    world.run_oneshot(|query: Query<(&Player, &Position, &Velocity)>| {
        let (_, position, velocity) = query.single();

        println!("Player position is {position:?} and velocity is {velocity:?}")
    });
}

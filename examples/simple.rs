use kecs::{Entity, GraphScheduler, KecsWorld, Query};

#[derive(Debug)]
struct EntityName(String);

struct Bullet {
    direction: [f32; 2],
}

// A struct without any members is called a tag structure
struct Player;

#[derive(Default, Debug)]
struct Transform {
    position: [f32; 2],
}

// A query is an iterator over the entities with the specified components
fn update_bullet_position(query: Query<(&Bullet, &mut Transform)>) {
    for (bullet, transform) in query.iter() {
        transform.position[0] += bullet.direction[0];
        transform.position[1] += bullet.direction[1];
    }
}

// You can also get the entity itself
fn print_transform_system(query: Query<(Entity, &EntityName, &Transform)>) {
    for (entity, entity_name, transform) in query.iter() {
        println!(
            "Transform of entity {entity:?} with name '{}' at {:?}",
            entity_name.0, transform
        );
    }
}

fn print_player_position(query: Query<(&Player, Entity, &EntityName, &Transform)>) {
    for (_, entity, entity_name, transform) in query.iter() {
        println!(
            "Transform of player {entity:?} with name '{}' at {:?}",
            entity_name.0, transform
        );
    }
}

fn main() {
    let mut world = KecsWorld::<GraphScheduler>::new();

    {
        let bullet_entity = world.new_entity();
        world.add_component(
            bullet_entity,
            Bullet {
                direction: [1.0, 0.0],
            },
        );
        world.add_component(bullet_entity, Transform::default());
        world.add_component(bullet_entity, EntityName("Bullet 0".to_owned()));
    }

    {
        let bullet_entity = world.new_entity();
        world.add_component(
            bullet_entity,
            Bullet {
                direction: [-1.0, 0.0],
            },
        );
        world.add_component(bullet_entity, Transform::default());
        world.add_component(bullet_entity, EntityName("Bullet 1".to_owned()));
    }

    {
        let player_entity = world.new_entity();
        world.add_component(player_entity, Player);
        world.add_component(player_entity, Transform::default());
        world.add_component(player_entity, EntityName("Player".to_owned()));
    }

    let label = 0;
    world.add_system(label, update_bullet_position);
    world.add_system(label, print_player_position);
    world.add_system(label, print_transform_system);

    // the GraphScheduler allows for printing the scheduled systems in Dot format
    world.scheduler(label).print_jobs();

    // In real code this should belong in an event loop
    for i in 0..3 {
        println!("Frame {i}");
        world.update(label);
        println!("\n\n");
    }
}

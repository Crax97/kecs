use std::sync::{atomic::AtomicU32, RwLock};

use crate::{ArchetypeId, ComponentId, SparseSet};

/// [`Entity`]s are the first building blocks of an ECS: they are used to associate one or more components together,
/// on which [`crate::System`]s operate: they are implemented as an integer, which uniquely identifies the components
/// associated to the Entity
#[derive(Default, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct Entity(pub(crate) u32, pub(crate) u32);

/// Holds all the informations about an entity, such as its ArchetypeId and the entity's components
#[derive(Default, Clone, Debug)]
pub struct EntityInfo {
    /// The entity's generation
    pub generation: u32,
    /// The set of all the components belonging to this [`Entity`]
    pub components: SparseSet<ComponentId, ()>,

    /// The [`ArchetypeId`] of this [`Entity`]
    pub archetype_id: ArchetypeId,
}

#[derive(Default)]
pub(crate) struct EntityAllocator {
    next_entity_id: AtomicU32,
    entity_info: SparseSet<Entity, EntityInfo>,
    dropped_entities: RwLock<Vec<Entity>>,
}

impl EntityAllocator {
    pub fn new_entity(&mut self) -> Entity {
        let id = self.allocate_id();
        self.new_with_id(id);
        id
    }

    pub fn allocate_id(&self) -> Entity {
        if let Some(mut entity) = self
            .dropped_entities
            .write()
            .expect("dropped_entities")
            .pop()
        {
            entity.1 += 1;
            return entity;
        }
        let id = self
            .next_entity_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Entity(id, 0)
    }

    pub fn new_with_id(&mut self, id: Entity) {
        assert!(!self.entity_info.contains(&id));
        self.entity_info.insert(
            id,
            EntityInfo {
                generation: id.1,
                ..Default::default()
            },
        );
    }

    pub fn destroy_entity(&mut self, entity: Entity) {
        self.entity_info.remove(entity);
        self.dropped_entities
            .write()
            .expect("dropped entityes")
            .push(entity);
    }

    pub fn entity_info(&self, id: Entity) -> Option<&EntityInfo> {
        self.entity_info
            .get(&id)
            .filter(|info| info.generation == id.1)
    }

    pub fn entity_info_mut(&mut self, id: Entity) -> Option<&mut EntityInfo> {
        self.entity_info
            .get_mut(id)
            .filter(|info| info.generation == id.1)
    }

    pub(crate) fn iter_all_entities(&self) -> impl Iterator<Item = (Entity, &EntityInfo)> + '_ {
        self.entity_info.iter()
    }
}

#[cfg(test)]
mod tests {
    use crate::{type_registrar::UniqueTypeId, ComponentId};

    use super::EntityAllocator;

    #[test]
    fn test_entity_allocator() {
        let mut allocator = EntityAllocator::default();

        let id = allocator.new_entity();
        assert!(allocator.entity_info(id).is_some());

        let dummy_component = ComponentId(UniqueTypeId(0, ""));
        allocator
            .entity_info_mut(id)
            .unwrap()
            .components
            .insert(dummy_component, ());

        allocator.destroy_entity(id);
        let new_id = allocator.new_entity();
        assert_eq!(new_id.0, id.0);
        assert_ne!(new_id.1, id.1);
        assert!(allocator.entity_info(id).is_none());
        assert!(allocator.entity_info(new_id).is_some());
        assert!(!allocator
            .entity_info(new_id)
            .unwrap()
            .components
            .contains(&dummy_component));
        assert!(allocator.entity_info(new_id).unwrap().generation == 1);
    }
}

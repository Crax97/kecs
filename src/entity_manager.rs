use std::sync::atomic::AtomicU32;

use crate::{ArchetypeId, ComponentId, SparseSet};

/// [`Entity`]s are the first building blocks of an ECS: they are used to associate one or more components together,
/// on which [`crate::System`]s operate: they are implemented as an integer, which uniquely identifies the components
/// associated to the Entity
#[derive(Default, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct Entity(pub(crate) u32);

/// Holds all the informations about an entity, such as its ArchetypeId and the entity's components
#[derive(Default, Clone, Debug)]
pub struct EntityInfo {
    /// The set of all the components belonging to this [`Entity`]
    pub components: SparseSet<ComponentId, ()>,

    /// The [`ArchetypeId`] of this [`Entity`]
    pub archetype_id: ArchetypeId,
}

#[derive(Default)]
pub(crate) struct EntityAllocator {
    next_entity_id: AtomicU32,
    entity_info: SparseSet<Entity, EntityInfo>,
    dropped_entities: Vec<Entity>,
}

impl EntityAllocator {
    pub fn new_entity(&mut self) -> Entity {
        let id = self.allocate_id();
        self.new_with_id(id);
        id
    }

    pub fn allocate_id(&self) -> Entity {
        let id = self
            .next_entity_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Entity(id)
    }

    pub fn new_with_id(&mut self, id: Entity) {
        assert!(!self.entity_info.contains(&id));
        self.entity_info.insert(id, EntityInfo::default());
    }

    pub fn destroy_entity(&mut self, entity: Entity) {
        self.entity_info.remove(entity);
        self.dropped_entities.push(entity);
    }

    pub fn entity_info(&self, id: Entity) -> Option<&EntityInfo> {
        self.entity_info.get(&id)
    }

    pub fn entity_info_mut(&mut self, id: Entity) -> Option<&mut EntityInfo> {
        self.entity_info.get_mut(id)
    }

    pub(crate) fn iter_all_entities(&self) -> impl Iterator<Item = (Entity, &EntityInfo)> + '_ {
        self.entity_info.iter()
    }
}

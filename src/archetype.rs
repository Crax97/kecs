use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};

use crate::{sparse_set::SparseSet, ComponentId, Entity};

#[derive(Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct ArchetypeId(usize);

#[derive(Clone, Debug, Default)]
pub struct Archetype {
    pub components: HashSet<ComponentId>,
    pub entities: SparseSet<Entity, ()>,
}

impl Archetype {
    /// Checks if other's keys are all included in self's keys
    pub fn includes_fully(&self, other: &Archetype) -> bool {
        for component in &other.components {
            if !self.components.contains(component) {
                return false;
            }
        }

        true
    }
}

#[derive(Default)]
pub struct ArchetypeManager {
    archetypes: HashMap<ArchetypeId, Archetype>,
}

impl ArchetypeManager {
    pub fn archetype_of<T>(&mut self, ids: &SparseSet<ComponentId, T>) -> ArchetypeId {
        let mut hasher = std::hash::DefaultHasher::new();
        for (component, _) in ids.iter() {
            component.hash(&mut hasher);
        }
        let id = hasher.finish();
        let id = ArchetypeId(id as usize);
        self.archetypes.entry(id).or_insert_with(|| Archetype {
            components: ids.iter().map(|(i, _)| i).collect(),
            entities: Default::default(),
        });
        id
    }

    pub fn get_archetype(&self, id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get(&id)
    }

    pub fn get_archetype_mut(&mut self, id: ArchetypeId) -> Option<&mut Archetype> {
        self.archetypes.get_mut(&id)
    }
}

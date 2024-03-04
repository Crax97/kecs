use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};

use crate::{sparse_set::SparseSet, ComponentId, Entity};

/// The unique id of an [`Archetype`]
#[derive(Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct ArchetypeId(usize);

/// An Archetype is an unique set of components
#[derive(Clone, Debug, Default)]
pub struct Archetype {
    /// The components that compose this archetype
    pub components: HashSet<ComponentId>,
    /// Which entities are associated to this archetype
    pub entities: SparseSet<Entity, ()>,
}

impl Archetype {
    /// Checks if `other`'s component set is included in `self`'s component set
    pub fn includes_fully(&self, other: &Archetype) -> bool {
        for component in &other.components {
            if !self.components.contains(component) {
                return false;
            }
        }

        true
    }
}

/// The [`ArchetypeManager`] is in charge of storing the ids of the [`Archetype`]s and managing them.
#[derive(Default)]
pub struct ArchetypeManager {
    archetypes: HashMap<ArchetypeId, Archetype>,
}

impl ArchetypeManager {
    /// Gets the archetype of this component set, creating it if it doesn't exists
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

    /// Gets the archetype with this id if it exists
    pub fn get_archetype(&self, id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get(&id)
    }

    pub(crate) fn get_archetype_mut(&mut self, id: ArchetypeId) -> Option<&mut Archetype> {
        self.archetypes.get_mut(&id)
    }
}

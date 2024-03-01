use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::path::Display;

use petgraph::data::Build;
use petgraph::dot::Dot;
use petgraph::graph::NodeIndex;
use petgraph::{Directed, Graph};

use crate::query::AccessMode;
use crate::sparse_set::SparseSet;
use crate::system::{IntoSystem, System};
use crate::{ComponentId, World};

/// # Safety
///   The implementer must ensure that:
///   1. All resource accesses must respect Rust's borrowing rules: only one mutable access can be present
///      for each component/resource (even across threads), and if there's any non mutable access
///      then no mutable access must be performed on the resource
pub unsafe trait Scheduler {
    type SystemId: Sized + Eq + PartialEq + Ord + PartialOrd + Hash + Copy + Clone + Debug;

    fn new() -> Self;
    fn add_system<ARGS, S: IntoSystem<ARGS>>(
        &mut self,
        world: &mut World,
        system: S,
    ) -> Self::SystemId;
    fn remove_system(&mut self, id: Self::SystemId);

    fn execute(&mut self, world: &mut World);
}

/// This scheduler runs all the systems on the same thread sequentially
#[derive(Default)]
pub struct LinearScheduler {
    systems: Vec<Option<Box<dyn System>>>,
}
pub struct GraphScheduler {
    current_dependencies: SparseSet<ComponentId, GraphResourceOwnership>,
    graph: Graph<SystemGraphNode, SystemGraphEdge, Directed>,
    root_node_idx: NodeIndex,
}

/// # Safety
///   Since the systems are scheduled to be run sequentially, only one system at a time can access the system's resources
unsafe impl Scheduler for LinearScheduler {
    type SystemId = usize;

    fn new() -> Self {
        Self::default()
    }

    fn add_system<ARGS, S: IntoSystem<ARGS>>(
        &mut self,
        world: &mut World,
        system: S,
    ) -> Self::SystemId {
        let id = self.systems.len();
        let mut system = system.into_system();
        system.init(world);

        self.systems.push(Some(Box::new(system)));
        id
    }

    fn remove_system(&mut self, id: Self::SystemId) {
        if let Some(sys) = self.systems.get_mut(id) {
            *sys = None;
        }
    }

    fn execute(&mut self, world: &mut World) {
        for system in self.systems.iter_mut().filter_map(|s| s.as_mut()) {
            system.run(world);
        }
    }
}

/// # Safety
///   Since the systems are scheduled to be run sequentially, only one system at a time can access the system's resources
unsafe impl Scheduler for GraphScheduler {
    type SystemId = NodeIndex;

    fn new() -> Self {
        let mut graph = Graph::default();
        let root_node = SystemGraphNode {
            system: None,
            dependencies: Default::default(),
        };
        let root_node_idx = graph.add_node(root_node);
        Self {
            current_dependencies: Default::default(),
            graph,
            root_node_idx,
        }
    }

    fn add_system<ARGS, S: IntoSystem<ARGS>>(
        &mut self,
        world: &mut World,
        system: S,
    ) -> Self::SystemId {
        let mut system = system.into_system();
        system.init(world);
        let system_dependencies = system.compute_dependencies(world);

        let system_node = SystemGraphNode {
            system: Some(Box::new(system)),
            dependencies: system_dependencies.clone(),
        };
        let node = self.graph.add_node(system_node);
        let mut node_dependencies = HashMap::<NodeIndex, SystemGraphEdge>::new();
        for (component, ownership) in self.current_dependencies.iter() {
            if let Some(sys_access) = system_dependencies.get(&component) {
                if *sys_access != ownership.access_mode {
                    // Dependency: the access mode changes
                    node_dependencies
                        .entry(ownership.system_node)
                        .or_default()
                        .changes
                        .push(SystemGraphChange {
                            component,
                            new_access_mode: *sys_access,
                        });
                }
            }
        }

        if node_dependencies.is_empty() {
            for (component, access) in system_dependencies.iter() {
                self.current_dependencies.insert(
                    component,
                    GraphResourceOwnership {
                        access_mode: *access,
                        system_node: node,
                    },
                );
            }
            self.graph
                .add_edge(self.root_node_idx, node, SystemGraphEdge::default());
        } else {
            for (owner, changes) in node_dependencies {
                for change in &changes.changes {
                    let dep = self.current_dependencies.get_mut(change.component).unwrap();
                    dep.access_mode = change.new_access_mode;
                    dep.system_node = node;
                }
                self.graph.add_edge(owner, node, changes);
            }
        }

        node
    }

    fn remove_system(&mut self, id: Self::SystemId) {
        // if let Some(sys) = self.systems.get_mut(id) {
        //     *sys = None;
        // }
    }

    fn execute(&mut self, world: &mut World) {
        let dot = Dot::new(&self.graph);
        println!("{}", dot);
        // for system in self.systems.iter_mut().filter_map(|s| s.as_mut()) {
        //     system.run(world);
        // }
    }
}

pub struct SystemGraphNode {
    system: Option<Box<dyn System>>,
    dependencies: SparseSet<ComponentId, AccessMode>,
}

impl std::fmt::Debug for SystemGraphNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemGraphNode")
            .field(
                "system",
                &self
                    .system
                    .as_ref()
                    .map_or_else(|| Cow::Borrowed("Root"), |sys| sys.get_name()),
            )
            .field("dependencies", &self.dependencies)
            .finish()
    }
}

#[derive(Debug)]
pub struct SystemGraphChange {
    component: ComponentId,
    new_access_mode: AccessMode,
}

#[derive(Default, Debug)]
pub struct SystemGraphEdge {
    pub changes: Vec<SystemGraphChange>,
}

pub struct GraphResourceOwnership {
    access_mode: AccessMode,
    system_node: NodeIndex,
}

impl std::fmt::Display for SystemGraphNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            &self
                .system
                .as_ref()
                .map(|s| s.get_name())
                .unwrap_or("Root".into()),
        )
    }
}

impl std::fmt::Display for SystemGraphEdge {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: Write compont names + accesses
        Ok(())
    }
}

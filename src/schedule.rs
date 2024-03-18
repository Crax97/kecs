use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;
use std::vec;

use petgraph::dot::Dot;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::{Directed, Graph};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::query::AccessMode;
use crate::sparse_set::SparseSet;
use crate::system::{IntoSystem, System};
use crate::{ComponentId, Entity, WorldContainer};

/// # Safety
///   The implementer must ensure that:
///   1. All resource accesses must respect Rust's borrowing rules: only one mutable access can be present
///      for each component/resource (even across threads), and if there's any non mutable access
///      then no mutable access must be performed on the resource
pub unsafe trait Scheduler: Default {
    /// This type identifies a system added to the Scheduler
    type SystemId: Sized + Eq + PartialEq + Ord + PartialOrd + Hash + Copy + Clone + Debug;

    /// Implement this function to add a new instance of the Scheduler
    fn new() -> Self;

    /// Implement this function to add a new system to the Scheduler
    fn add_system<ARGS, S: IntoSystem<ARGS>>(
        &mut self,
        world: &mut WorldContainer,
        system: S,
    ) -> Self::SystemId;

    /// Implement this function to run the scheduler systems
    fn execute(&mut self, world: &mut WorldContainer);

    /// This method will be called when a new entity changes somehow (e.g an entity is created,
    /// a component is added/removed etc...)
    fn on_entity_updated(&mut self, world: &mut WorldContainer, entity: Entity);
}

/// This scheduler runs all the systems on the same thread sequentially
#[derive(Default)]
pub struct LinearScheduler {
    systems: Vec<Box<dyn System>>,
}

/// The [`GraphScheduler`] will put the systems into a graph where the nodes are the systems and
/// the edges are the dependencies between each system: this allows the systems to be run in parallel when possible
pub struct GraphScheduler {
    current_dependencies: SparseSet<ComponentId, GraphResourceOwnership>,
    graph: Graph<SystemGraphNode, SystemGraphEdge, Directed>,
    root_node_idx: NodeIndex,
    changed_schedule: bool,
    cached_schedule: Schedules,
}

impl Default for GraphScheduler {
    fn default() -> Self {
        Self::new()
    }
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
        world: &mut WorldContainer,
        system: S,
    ) -> Self::SystemId {
        let id = self.systems.len();
        let mut system = system.into_system();
        system.init(world);

        self.systems.push(Box::new(system));
        id
    }

    fn execute(&mut self, world: &mut WorldContainer) {
        for system in self.systems.iter_mut() {
            system.run(world);
        }
    }

    fn on_entity_updated(&mut self, world: &mut WorldContainer, entity: Entity) {
        if let Some(info) = world.get_entity_info(entity) {
            self.systems
                .iter_mut()
                .for_each(|s| s.on_entity_changed(world, entity, info))
        } else {
            self.systems
                .iter_mut()
                .for_each(|s| s.on_entity_destroyed(world, entity))
        }
    }
}

/// # Safety
/// The scheduler will run systems in parallel only when their dependencies (or access modes) don't overlap
/// Otherwise, the systems will be run sequentially
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
            changed_schedule: true,
            cached_schedule: Default::default(),
        }
    }

    fn add_system<ARGS, S: IntoSystem<ARGS>>(
        &mut self,
        world: &mut WorldContainer,
        system: S,
    ) -> Self::SystemId {
        let mut system = system.into_system();
        system.init(world);

        let system_is_exclusive = system.is_exclusive(world);

        let system_dependencies = system.compute_dependencies(world);

        let system_node = SystemGraphNode {
            system: Some(Box::new(system)),
            dependencies: system_dependencies.clone(),
        };
        let system_node_idx = self.graph.add_node(system_node);

        if system_is_exclusive {
            // If a system is exclusive, place a dependency on all the leaf nodes
            self.place_system_dependency_on_leaves(system_node_idx);
        } else {
            let node_dependencies = self.compute_node_dependencies(&system_dependencies);

            if node_dependencies.is_empty() {
                // System writes to a set of components never encountered before, place it at the beginning
                self.place_system_at_graph_begin(system_dependencies, system_node_idx);
            } else {
                self.place_system_dependencies(node_dependencies, system_node_idx);
            }
        }

        self.changed_schedule = true;
        system_node_idx
    }

    fn execute(&mut self, world: &mut WorldContainer) {
        #[derive(Clone)]
        struct SystemPtr(*mut dyn System);

        unsafe impl Send for SystemPtr {}
        unsafe impl Sync for SystemPtr {}

        if self.changed_schedule {
            self.cached_schedule = self.compute_schedule();
            self.changed_schedule = false;
        }
        for schedule in self.cached_schedule.groups.iter() {
            let world_ptr = unsafe { world.get_mut_ptr() };
            let job_ptrs =
                schedule
                    .jobs
                    .iter()
                    .map(|&job| {
                        let system = self.graph.node_weight_mut(job).unwrap();
                        SystemPtr(
                            system.system.as_mut().map(|m| m.as_mut()).unwrap() as *mut dyn System
                        )
                    })
                    .collect::<Vec<_>>();

            // SAFETY: All the pointers point to different systems, and the scheduler took care of ensuring
            // that systems in this schedule don't write to the same resources
            let exec_system = |sys: &SystemPtr| unsafe {
                let system = sys.0.as_mut().unwrap();
                system.run(world_ptr.copied().get_mut())
            };
            if cfg!(miri) {
                // Don't use rayon with miri, since the global rayon pool is never destroyed
                // even after the main thread exits (miri complains about that)
                job_ptrs.iter().for_each(exec_system)
            } else {
                job_ptrs.par_iter().for_each(exec_system)
            }
        }
    }

    fn on_entity_updated(&mut self, world: &mut WorldContainer, entity: Entity) {
        if let Some(info) = world.get_entity_info(entity) {
            self.graph.node_weights_mut().for_each(|s| {
                if let Some(s) = &mut s.system {
                    s.on_entity_changed(world, entity, info)
                }
            })
        } else {
            self.graph.node_weights_mut().for_each(|s| {
                if let Some(s) = &mut s.system {
                    s.on_entity_destroyed(world, entity)
                }
            })
        }
    }
}

impl GraphScheduler {
    fn compute_schedule(&self) -> Schedules {
        let mut previous_scheduled_nodes = HashSet::new();
        previous_scheduled_nodes.insert(self.root_node_idx);
        let mut current_jobs: HashSet<NodeIndex> = self
            .graph
            .edges(self.root_node_idx)
            .map(|e| e.target())
            .collect();
        let mut schedules = vec![];

        while !current_jobs.is_empty() {
            let mut next_jobs = HashSet::new();
            let mut current_schedule = vec![];
            for job in current_jobs {
                let mut parents = self
                    .graph
                    .edges_directed(job, petgraph::Direction::Incoming);
                let all_parents_scheduled =
                    parents.all(|p| previous_scheduled_nodes.contains(&p.source()));

                // A system can only be scheduled if all of its parents have been scheduled
                if all_parents_scheduled {
                    self.graph.edges(job).map(|e| e.target()).for_each(|j| {
                        next_jobs.insert(j);
                    });
                    current_schedule.push(job);
                    previous_scheduled_nodes.insert(job);
                }
            }

            if !current_schedule.is_empty() {
                schedules.push(Schedule {
                    jobs: current_schedule,
                })
            }
            current_jobs = next_jobs;
        }

        Schedules { groups: schedules }
    }

    fn place_system_dependency_on_leaves(&mut self, system_node_idx: NodeIndex) {
        let leaves: HashSet<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&node| {
                node != system_node_idx
                    && self
                        .graph
                        .edges_directed(node, petgraph::Direction::Outgoing)
                        .next()
                        .is_none()
            })
            .collect();
        for leaf in leaves {
            self.graph
                .add_edge(leaf, system_node_idx, SystemGraphEdge { changes: vec![] });
        }

        // Force every next resource to be scheduled after the exclusive system
        for dep in self.current_dependencies.iter_mut() {
            dep.access_mode = AccessMode::Write;
            dep.last_accessing.clear();
            dep.last_writing = Some(system_node_idx);
        }
    }

    fn compute_node_dependencies(
        &mut self,
        system_dependencies: &SparseSet<ComponentId, AccessMode>,
    ) -> HashMap<NodeIndex, SystemGraphEdge> {
        let mut node_dependencies = HashMap::<NodeIndex, SystemGraphEdge>::new();
        for (component, &access) in system_dependencies.iter() {
            let ownership = self.current_dependencies.get_mut(component);
            if let Some(ownership) = ownership {
                match access {
                    // If a system writes to a resource, it depends on the previous ones that read it.
                    // If no one read it, it depends on the last writing one
                    AccessMode::Write => {
                        // No previous reader, depend on the latest writing
                        if ownership.last_accessing.is_empty() {
                            if let Some(writer) = ownership.last_writing {
                                node_dependencies.insert(
                                    writer,
                                    SystemGraphEdge {
                                        changes: vec![SystemGraphChange {
                                            component,
                                            new_access_mode: access,
                                        }],
                                    },
                                );
                            }
                        } else {
                            // Depend on the latest readers
                            for reading in &ownership.last_accessing {
                                node_dependencies.insert(
                                    *reading,
                                    SystemGraphEdge {
                                        changes: vec![SystemGraphChange {
                                            component,
                                            new_access_mode: access,
                                        }],
                                    },
                                );
                            }
                        }
                    }
                    // If a system reads a resource, it depends on the latest one writing it
                    AccessMode::Read => {
                        if let Some(writer) = ownership.last_writing {
                            node_dependencies.insert(
                                writer,
                                SystemGraphEdge {
                                    changes: vec![SystemGraphChange {
                                        component,
                                        new_access_mode: access,
                                    }],
                                },
                            );
                        }
                    }
                }
            }
        }
        node_dependencies
    }

    fn place_system_at_graph_begin(
        &mut self,
        system_dependencies: SparseSet<ComponentId, AccessMode>,
        system_node_idx: NodeIndex,
    ) {
        for (component, access) in system_dependencies.iter() {
            self.current_dependencies.insert(
                component,
                GraphResourceOwnership {
                    access_mode: *access,
                    last_accessing: if *access == AccessMode::Read {
                        HashSet::from_iter([system_node_idx])
                    } else {
                        HashSet::default()
                    },
                    last_writing: if *access == AccessMode::Write {
                        Some(system_node_idx)
                    } else {
                        None
                    },
                },
            );
        }
        self.graph.add_edge(
            self.root_node_idx,
            system_node_idx,
            SystemGraphEdge::default(),
        );
    }

    fn place_system_dependencies(
        &mut self,
        node_dependencies: HashMap<NodeIndex, SystemGraphEdge>,
        system_node_idx: NodeIndex,
    ) {
        for (owner, changes) in node_dependencies {
            for change in &changes.changes {
                let dep = self.current_dependencies.get_mut(change.component).unwrap();
                dep.access_mode = change.new_access_mode;

                if change.new_access_mode == AccessMode::Read {
                    dep.last_accessing.insert(system_node_idx);
                } else {
                    dep.last_accessing.clear();
                    dep.last_writing = Some(system_node_idx);
                }
            }
            self.graph.add_edge(owner, system_node_idx, changes);
        }
    }
}

#[derive(Default, Debug)]
struct Schedule {
    jobs: Vec<NodeIndex>,
}

#[derive(Default, Debug)]
struct Schedules {
    groups: Vec<Schedule>,
}

impl GraphScheduler {
    /// This method prints the current job graph to stdout in Dot format, which can be viewed e.g
    /// using [https://viz-js.com/](https://viz-js.com/)
    pub fn print_jobs(&self) {
        let dot = Dot::new(&self.graph);
        println!("{}", dot);
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

#[derive(Debug)]
pub struct GraphResourceOwnership {
    access_mode: AccessMode,

    // If a system reads a component, it depends on the last one writing it
    last_writing: Option<NodeIndex>,

    // If a system needs to write a component, it depends on the last ones accessing the component
    last_accessing: HashSet<NodeIndex>,
}

impl std::fmt::Display for SystemGraphNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            &self
                .system
                .as_ref()
                .map(|s| s.get_name())
                .unwrap_or("Root".into()),
        )?;

        if !self.dependencies.is_empty() {
            writeln!(f)?;
        }

        for (component, access_mode) in self.dependencies.iter() {
            if *access_mode == AccessMode::Read {
                f.write_fmt(format_args!("R({}),", &component.name()))?;
            } else {
                f.write_fmt(format_args!("W({}),", &component.name()))?;
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for SystemGraphEdge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for SystemGraphChange {
            component,
            new_access_mode,
        } in &self.changes
        {
            if *new_access_mode == AccessMode::Read {
                f.write_fmt(format_args!("R({})", &component.name()))?;
            } else {
                f.write_fmt(format_args!("W({})", &component.name()))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{commands::Commands, query::Query, Entity, WorldContainer};

    use super::{GraphScheduler, Scheduler};

    #[derive(Default)]
    struct Component1;

    #[derive(Default)]
    struct Component2;

    fn make_world_container() -> WorldContainer {
        WorldContainer::new(Commands::create().0)
    }

    fn write_component_1(_: Query<&mut Component1>) {}
    fn write_component_2(_: Query<&mut Component2>) {}

    fn read_component_1(_: Query<&Component1>) {}
    fn non_parallel_system(_: &mut WorldContainer) {}
    fn read_write_component_1(_: Query<&Component1>, _: Query<&mut Component1>) {}

    #[test]
    fn empty_schedule() {
        let scheduler = GraphScheduler::new();

        let schedule = scheduler.compute_schedule();
        assert!(schedule.groups.is_empty());
    }

    #[test]
    fn write_then_read() {
        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let system_0 = scheduler.add_system(&mut world, write_component_1);
        let system_1 = scheduler.add_system(&mut world, write_component_2);

        let schedule = scheduler.compute_schedule();
        assert_eq!(schedule.groups.len(), 1);
        assert!(schedule.groups[0].jobs.contains(&system_0));
        assert!(schedule.groups[0].jobs.contains(&system_1));
    }

    #[test]
    fn disjoint_systems() {
        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let system_0 = scheduler.add_system(&mut world, write_component_1);
        let system_1 = scheduler.add_system(&mut world, read_component_1);

        scheduler.print_jobs();

        let schedule = scheduler.compute_schedule();
        assert_eq!(schedule.groups.len(), 2);
        assert!(schedule.groups[0].jobs.contains(&system_0));
        assert!(schedule.groups[1].jobs.contains(&system_1));
    }

    #[test]
    fn parallel_read() {
        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let system_0 = scheduler.add_system(&mut world, read_component_1);
        let system_1 = scheduler.add_system(&mut world, read_component_1);

        let schedule = scheduler.compute_schedule();
        assert_eq!(schedule.groups.len(), 1);
        assert!(schedule.groups[0].jobs.contains(&system_0));
        assert!(schedule.groups[0].jobs.contains(&system_1));
    }

    #[test]
    fn parallel_write() {
        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let system_0 = scheduler.add_system(&mut world, write_component_1);
        let system_1 = scheduler.add_system(&mut world, write_component_1);

        let schedule = scheduler.compute_schedule();
        assert_eq!(schedule.groups.len(), 2);
        assert!(schedule.groups[0].jobs.contains(&system_0));
        assert!(schedule.groups[1].jobs.contains(&system_1));
    }

    #[test]
    fn read_then_write() {
        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let system_0 = scheduler.add_system(&mut world, read_component_1);
        let system_1 = scheduler.add_system(&mut world, write_component_1);

        let schedule = scheduler.compute_schedule();
        assert_eq!(schedule.groups.len(), 2);
        assert!(schedule.groups[0].jobs.contains(&system_0));
        assert!(schedule.groups[1].jobs.contains(&system_1));
    }

    #[test]
    fn read_then_write_same_query() {
        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let system_0 = scheduler.add_system(&mut world, read_component_1);

        // Since this system writes TestComponent in a query, system_2 must wait for it
        let system_1 = scheduler.add_system(&mut world, read_write_component_1);

        let system_2 = scheduler.add_system(&mut world, read_component_1);

        let schedule = scheduler.compute_schedule();

        scheduler.print_jobs();
        assert_eq!(schedule.groups.len(), 3);

        assert!(schedule.groups[0].jobs.contains(&system_0));
        assert!(schedule.groups[1].jobs.contains(&system_1));
        assert!(schedule.groups[2].jobs.contains(&system_2));
    }

    #[test]
    fn non_parallel_world() {
        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let system_0 = scheduler.add_system(&mut world, read_component_1);

        // If a system takes a &mut World, it's an exclusive system: it cannot be run in parallel in any case
        let system_1 = scheduler.add_system(&mut world, non_parallel_system);

        let schedule = scheduler.compute_schedule();

        scheduler.print_jobs();
        assert_eq!(schedule.groups.len(), 2);

        assert!(schedule.groups[0].jobs.contains(&system_0));
        assert!(schedule.groups[1].jobs.contains(&system_1));
    }

    /// System A, B, C read from the same component but write to different components
    /// then F writes to the world
    /// Finally D uses A's result with a non-send resource
    /// The schedule should be (A, B, C) -> (F) -> (D)
    #[test]
    fn multi_nodes() {
        struct SharedByABC;
        struct WrittenByA;
        struct WrittenByB;
        struct WrittenByC;

        fn sys_a(_: Query<(&SharedByABC, &mut WrittenByA)>) {}
        fn sys_b(_: Query<(&SharedByABC, &mut WrittenByB)>) {}
        fn sys_c(_: Query<(&SharedByABC, &mut WrittenByC)>) {}
        fn exclusive_sys(_: &mut WorldContainer) {}
        fn sys_d(_: &mut WorldContainer) {}
        fn sys_e(_: Query<&WrittenByA>, _: &WorldContainer) {}
        fn sys_f(_: Query<(&mut WrittenByB, &WrittenByC)>, _: &WorldContainer) {}

        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let sys_a_id = scheduler.add_system(&mut world, sys_a);
        let sys_b_id = scheduler.add_system(&mut world, sys_b);
        let sys_c_id = scheduler.add_system(&mut world, sys_c);
        let sys_excl_id = scheduler.add_system(&mut world, exclusive_sys);
        let sys_d_id = scheduler.add_system(&mut world, sys_d);
        let sys_e_id = scheduler.add_system(&mut world, sys_e);
        let sys_f_id = scheduler.add_system(&mut world, sys_f);

        let schedule = scheduler.compute_schedule();

        scheduler.print_jobs();
        assert_eq!(schedule.groups.len(), 4);

        assert!(schedule.groups[0].jobs.contains(&sys_a_id));
        assert!(schedule.groups[0].jobs.contains(&sys_b_id));
        assert!(schedule.groups[0].jobs.contains(&sys_c_id));
        assert!(schedule.groups[1].jobs.contains(&sys_excl_id));
        assert!(schedule.groups[2].jobs.contains(&sys_d_id));
        assert!(schedule.groups[3].jobs.contains(&sys_e_id));
        assert!(schedule.groups[3].jobs.contains(&sys_f_id));
    }

    #[test]
    fn game() {
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

        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();
        let update = scheduler.add_system(&mut world, update_bullet_position);
        let print_1 = scheduler.add_system(&mut world, print_player_position);
        let print_2 = scheduler.add_system(&mut world, print_transform_system);
        scheduler.print_jobs();

        let schedule = scheduler.compute_schedule();

        assert!(schedule.groups.len() == 2);
        assert!(schedule.groups[0].jobs.contains(&update));
        assert!(schedule.groups[1].jobs.contains(&print_1));
        assert!(schedule.groups[1].jobs.contains(&print_2));
    }

    #[test]
    fn write_read_write() {
        struct TestComponentA;

        fn sys_write_a(_: Query<&mut TestComponentA>) {}
        fn sys_read_a(_: Query<&TestComponentA>) {}

        let mut world = make_world_container();
        let mut scheduler = GraphScheduler::new();

        let sys_1 = scheduler.add_system(&mut world, sys_write_a);
        let sys_2 = scheduler.add_system(&mut world, sys_read_a);
        let sys_3 = scheduler.add_system(&mut world, sys_read_a);
        let sys_4 = scheduler.add_system(&mut world, sys_write_a);
        let sys_5 = scheduler.add_system(&mut world, sys_write_a);

        let schedule = scheduler.compute_schedule();

        scheduler.print_jobs();

        assert!(schedule.groups.len() == 4);
        assert!(schedule.groups[0].jobs.len() == 1 && schedule.groups[0].jobs.contains(&sys_1));
        assert!(schedule.groups[1].jobs.len() == 2 && schedule.groups[1].jobs.contains(&sys_2));
        assert!(schedule.groups[1].jobs.len() == 2 && schedule.groups[1].jobs.contains(&sys_3));
        assert!(schedule.groups[2].jobs.len() == 1 && schedule.groups[2].jobs.contains(&sys_4));
        assert!(schedule.groups[3].jobs.len() == 1 && schedule.groups[3].jobs.contains(&sys_5));
    }
}

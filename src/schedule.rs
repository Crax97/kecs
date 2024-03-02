use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;

use petgraph::dot::Dot;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
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

    fn execute(&mut self, world: &mut World);
}

/// This scheduler runs all the systems on the same thread sequentially
#[derive(Default)]
pub struct LinearScheduler {
    systems: Vec<Box<dyn System>>,
}
pub struct GraphScheduler {
    current_dependencies: SparseSet<ComponentId, GraphResourceOwnership>,
    graph: Graph<SystemGraphNode, SystemGraphEdge, Directed>,
    root_node_idx: NodeIndex,
    changed_schedule: bool,
    cached_schedule: Schedules,
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

        self.systems.push(Box::new(system));
        id
    }

    fn execute(&mut self, world: &mut World) {
        for system in self.systems.iter_mut() {
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
            changed_schedule: true,
            cached_schedule: Default::default(),
        }
    }

    fn add_system<ARGS, S: IntoSystem<ARGS>>(
        &mut self,
        world: &mut World,
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
        } else {
            let mut node_dependencies = HashMap::<NodeIndex, SystemGraphEdge>::new();
            for (component, ownership) in self.current_dependencies.iter() {
                if let Some(sys_access) = system_dependencies.get(&component) {
                    if *sys_access != ownership.access_mode
                        || (*sys_access == AccessMode::Write
                            && ownership.access_mode == AccessMode::Write)
                    {
                        // Dependency: the access mode changes or the previous system writes to the same components
                        // of the new system
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
                            system_node: system_node_idx,
                        },
                    );
                }
                self.graph.add_edge(
                    self.root_node_idx,
                    system_node_idx,
                    SystemGraphEdge::default(),
                );
            } else {
                for (owner, changes) in node_dependencies {
                    for change in &changes.changes {
                        let dep = self.current_dependencies.get_mut(change.component).unwrap();
                        dep.access_mode = change.new_access_mode;
                        dep.system_node = system_node_idx;
                    }
                    self.graph.add_edge(owner, system_node_idx, changes);
                }
            }
        }
        self.changed_schedule = true;
        system_node_idx
    }

    fn execute(&mut self, world: &mut World) {
        if self.changed_schedule {
            self.cached_schedule = self.compute_schedule();
            self.changed_schedule = false;
        }
        for (i, schedule) in self.cached_schedule.groups.iter().enumerate() {
            println!("Schedule {i}");
            for job in &schedule.jobs {
                let system = self.graph.node_weight_mut(*job).unwrap();
                if let Some(system) = &mut system.system {
                    println!("\tScheduling job {}", system.get_name());
                    system.run(world);
                }
            }
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
}

#[derive(Default)]
struct Schedule {
    jobs: Vec<NodeIndex>,
}

#[derive(Default)]
struct Schedules {
    groups: Vec<Schedule>,
}

impl GraphScheduler {
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

#[cfg(test)]
mod tests {

    use crate::{
        query::Query,
        resources::{ResMut, Resource},
        World,
    };

    use super::{GraphScheduler, Scheduler};

    #[derive(Default)]
    struct Component1;

    #[derive(Default)]
    struct Component2;

    fn write_component_1(_: Query<&mut Component1>) {}
    fn write_component_2(_: Query<&mut Component2>) {}

    fn read_component_1(_: Query<&Component1>) {}
    fn read_component_2(_: Query<&Component2>) {}
    fn non_parallel_system(_: &mut World, _: Query<&Component1>) {}
    fn read_write_component_1(_: Query<&Component1>, _: Query<&mut Component1>) {}

    #[test]
    fn empty_schedule() {
        let scheduler = GraphScheduler::new();

        let schedule = scheduler.compute_schedule();
        assert!(schedule.groups.is_empty());
    }

    #[test]
    fn write_then_read() {
        let mut world = World::new();
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
        let mut world = World::new();
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
        let mut world = World::new();
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
        let mut world = World::new();
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
        let mut world = World::new();
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
        let mut world = World::new();
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
        let mut world = World::new();
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

        struct NonSendResource;
        impl Resource for NonSendResource {}

        fn sys_a(_: Query<(&SharedByABC, &mut WrittenByA)>) {}
        fn sys_b(_: Query<(&SharedByABC, &mut WrittenByB)>) {}
        fn sys_c(_: Query<(&SharedByABC, &mut WrittenByC)>) {}
        fn exclusive_sys(_: &mut World) {}
        fn sys_d(_: Query<&WrittenByA>, _: ResMut<NonSendResource>) {}

        let mut world = World::new();
        let mut scheduler = GraphScheduler::new();

        // Register  first the resource
        world.add_non_send_resource(NonSendResource);

        let sys_a_id = scheduler.add_system(&mut world, sys_a);
        let sys_b_id = scheduler.add_system(&mut world, sys_b);
        let sys_c_id = scheduler.add_system(&mut world, sys_c);
        let sys_f_id = scheduler.add_system(&mut world, exclusive_sys);
        let sys_d_id = scheduler.add_system(&mut world, sys_d);

        let schedule = scheduler.compute_schedule();

        scheduler.print_jobs();
        assert_eq!(schedule.groups.len(), 3);

        assert!(schedule.groups[0].jobs.contains(&sys_a_id));
        assert!(schedule.groups[0].jobs.contains(&sys_b_id));
        assert!(schedule.groups[0].jobs.contains(&sys_c_id));
        assert!(schedule.groups[1].jobs.contains(&sys_f_id));
        assert!(schedule.groups[2].jobs.contains(&sys_d_id));
    }
}

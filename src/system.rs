use crate::{
    erased_data_vec::ErasedVec,
    query::{AccessMode, Query, QueryParam, QueryState},
    resources::{Res, ResMut, Resource},
    sparse_set::SparseSet,
    ComponentId, Entity, EntityInfo, WorldContainer,
};
use std::{borrow::Cow, marker::PhantomData};

/// The trait used to identify all the types that can be used as system parameters
/// (e.g [`Query`], [`Res`]/[`ResMut`]).
pub trait SystemParam: Sized {
    /// The state used by this parameter
    type State: Send + Sync + 'static;

    /// `true` if the parameter is `&mut WorldContainer`
    const IS_WORLD: bool;

    /// This method is used to add this parameter's dependencies to the `components` [`SparseSet`]
    fn add_dependencies(
        store: &mut WorldContainer,
        components: &mut SparseSet<ComponentId, AccessMode>,
    );

    /// This method is used to create the parameter
    fn create<'world, 'state>(data: &'state Self::State, store: &'world mut WorldContainer) -> Self
    where
        'world: 'state;

    /// This method is used to create the parameter's [`Self::State`]
    fn create_initial_state(store: &mut WorldContainer) -> Self::State;

    /// This method is called when the state of an [`Entity`] changes (e.g a new [`Entity`] is created/a new component is added etc...)
    fn on_entity_changed(
        state: &mut Self::State,
        store: &WorldContainer,
        entity: Entity,
        info: &EntityInfo,
    );

    /// This method should return true if the parameter exclusively accesses a parameter
    fn is_exclusive(world: &WorldContainer) -> bool;
}

/// The trait implemented by all systems, which can be added into a [`crate::Scheduler`].
pub trait System: Send + Sync + 'static {
    /// Gets the system's name
    fn get_name(&self) -> Cow<'static, str>;

    /// Called when the system is added into a [`crate::Scheduler`]
    fn init(&mut self, store: &mut WorldContainer);

    /// Called to execute the system
    fn run(&mut self, store: &mut WorldContainer);

    /// Called to get the system's dependencies
    fn compute_dependencies(
        &self,
        world: &mut WorldContainer,
    ) -> SparseSet<ComponentId, AccessMode>;

    /// Called when the state of an entity changes
    fn on_entity_changed(&mut self, store: &WorldContainer, entity: Entity, info: &EntityInfo);

    /// Must return true if the system should be scheduled on the main thread
    fn is_exclusive(&self, world: &WorldContainer) -> bool;
}

pub trait IntoSystem<ARGS> {
    type SystemType: System;
    const HAS_WORLD: bool;
    const NUM_PARAMS: usize;

    fn into_system(self) -> Self::SystemType;
}

impl<'qworld, 'qstate, A: QueryParam> SystemParam for Query<'qworld, 'qstate, A> {
    type State = QueryState;
    const IS_WORLD: bool = false;

    fn add_dependencies(
        store: &mut WorldContainer,
        components: &mut SparseSet<ComponentId, AccessMode>,
    ) {
        A::compute_component_set(store, components);
    }
    fn create<'world, 'state>(data: &'state Self::State, store: &'world mut WorldContainer) -> Self
    where
        'world: 'state,
    {
        // SAFETY: We know that 'world: 'state, so we should be good to go
        unsafe { std::mem::transmute(Query::<'_, '_, A>::create_query(data, store.get_mut_ptr())) }
    }

    fn create_initial_state(store: &mut WorldContainer) -> Self::State {
        let mut component_set = Default::default();
        Self::add_dependencies(store, &mut component_set);

        let state = QueryState {
            query_archetype: store
                .get_archetype_manager_mut()
                .archetype_of(&component_set),
            ..Default::default()
        };
        state
    }

    fn on_entity_changed(
        state: &mut Self::State,
        store: &WorldContainer,
        entity: Entity,
        info: &EntityInfo,
    ) {
        let archetype_manager = store.get_archetype_manager();
        let system_archetype = archetype_manager
            .get_archetype(state.query_archetype)
            .expect("Failed to get system archetype");
        let entity_archetype = archetype_manager
            .get_archetype(info.archetype_id)
            .expect("Failed to get archetype for entity");
        if entity_archetype.includes_fully(system_archetype) {
            state.entities.insert(entity);
        } else {
            state.entities.remove(&entity);
        }
    }

    fn is_exclusive(_world: &WorldContainer) -> bool {
        false
    }
}

impl SystemParam for &mut WorldContainer {
    type State = ();
    const IS_WORLD: bool = true;

    fn add_dependencies(
        store: &mut WorldContainer,
        components: &mut SparseSet<ComponentId, AccessMode>,
    ) {
        let id_of_world = store.get_or_create_component_id::<WorldContainer>();
        components.insert(id_of_world, AccessMode::Write);
    }

    fn create<'world, 'state>(_data: &'state Self::State, store: &'world mut WorldContainer) -> Self
    where
        'world: 'state,
    {
        unsafe { std::mem::transmute(store) }
    }

    fn create_initial_state(_store: &mut WorldContainer) -> Self::State {}

    fn on_entity_changed(
        _state: &mut Self::State,
        _store: &WorldContainer,
        _entity: Entity,
        _info: &EntityInfo,
    ) {
    }

    fn is_exclusive(_world: &WorldContainer) -> bool {
        true
    }
}

/// Wrapper type for a `fn` system
pub struct SystemContainer<F, A> {
    _args: PhantomData<A>,
    fun: F,
    system_data: Vec<ErasedVec>,
    fun_name: Cow<'static, str>,
}

impl<F, A> SystemContainer<F, A> {
    pub(crate) fn new(fun: F, name: Cow<'static, str>) -> Self {
        Self {
            _args: PhantomData,
            fun,
            system_data: vec![],
            fun_name: name,
        }
    }
}

impl<F, A> Drop for SystemContainer<F, A> {
    fn drop(&mut self) {
        for data in &mut self.system_data {
            unsafe {
                data.drop_at(0);
            }
        }
    }
}

macro_rules! impl_system {
    ($($param:ident:$idx:expr)*) => {
        impl<$($param: SystemParam + Send + Sync + 'static,)* FUN: Fn($($param,)*) + Send + Sync + 'static> System
            for SystemContainer<FUN, ($($param,)*)>
        {

            fn get_name(&self) -> Cow<'static, str> {
                self.fun_name.clone()
            }

            fn init(&mut self, store: &mut WorldContainer) {
                $(
                self.system_data.push({
                    let mut erased = unsafe { ErasedVec::new_typed::<$param::State>(true, 1) };
                    let data = $param::create_initial_state(store);
                    unsafe { erased.push_back(data) };
                    erased
                });
                )*

                for (entity, info) in store.iter_all_entities() {
                    self.on_entity_changed(store, entity, info)
                }
            }

            fn run(&mut self, store: &mut WorldContainer) {
                (self.fun)($($param::create(unsafe { self.system_data[$idx].get::<$param::State>(0) }, store),)*);
            }

            fn on_entity_changed(&mut self, store: &WorldContainer, entity: Entity, info: &EntityInfo) {
                {
                    $(
                        let state = unsafe { self.system_data[$idx].get_mut::<$param::State>(0) };
                        $param::on_entity_changed(state, store, entity, info);
                    )*
                }
            }

            fn compute_dependencies(&self, world: &mut WorldContainer) -> SparseSet<ComponentId, AccessMode> {
                let mut deps = Default::default();
                $(
                {
                    let mut param_deps = Default::default();
                    $param::add_dependencies(world, &mut param_deps);
                    add_dependencies(param_deps, &mut deps);
                }
                )*
                deps
            }

            fn is_exclusive(&self, world: &WorldContainer) -> bool
            {
                $(
                    $param::is_exclusive(world) ||
                )* false
            }
        }

        impl<$($param,)* FUN: Fn($($param,)*) + Send + Sync + 'static> IntoSystem<($($param,)*)> for FUN
        where
            $($param: SystemParam + Send + Sync + 'static,)*
        {
            const HAS_WORLD : bool = $( $param::IS_WORLD || ) * false;
            const NUM_PARAMS: usize = $(count_params::<$param>() + )* 0;

            type SystemType = SystemContainer<FUN, ($($param,)*)>;

            fn into_system(self) -> Self::SystemType {
                if Self::HAS_WORLD && Self::NUM_PARAMS > 1 {
                    panic!("If a system has a parameter of &mut WorldContainer, then that parameter must be the only parameter");
                }

                SystemContainer::new(self, Cow::Borrowed(std::any::type_name::<FUN>()))
            }
        }
    };
}

impl_system!(A:0);
impl_system!(A:0 B:1);
impl_system!(A:0 B:1 C:2);
impl_system!(A:0 B:1 C:2 D:3);
impl_system!(A:0 B:1 C:2 D:3 E:4);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9 K:10);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9 K:10 L:11);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9 K:10 L:11 M:12);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9 K:10 L:11 M:12 N:13);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9 K:10 L:11 M:12 N:13 O:14);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9 K:10 L:11 M:12 N:13 O:14 P:15);
impl_system!(A:0 B:1 C:2 D:3 E:4 F:5 G:6 H:7 I:8 J:9 K:10 L:11 M:12 N:13 O:14 P:15 Q:16);

#[allow(clippy::extra_unused_type_parameters)]
const fn count_params<A>() -> usize {
    1
}

fn add_dependencies(
    param_deps: SparseSet<ComponentId, AccessMode>,
    system_deps: &mut SparseSet<ComponentId, AccessMode>,
) {
    for (component, access) in param_deps.iter() {
        if let Some(sys_access) = system_deps.get_mut(component) {
            if *access == AccessMode::Write {
                *sys_access = *access;
            }
        } else {
            system_deps.insert(component, *access);
        }
    }
}

impl<'rworld, 'res, R: Resource + Send + Sync + 'static> SystemParam for Res<'rworld, 'res, R> {
    type State = ();
    const IS_WORLD: bool = false;

    fn add_dependencies(
        store: &mut WorldContainer,
        components: &mut SparseSet<ComponentId, AccessMode>,
    ) {
        let id = store.get_component_id_assertive::<R>();
        components.insert(id, AccessMode::Read);
    }

    fn create<'world, 'state>(_data: &'state Self::State, store: &'world mut WorldContainer) -> Self
    where
        'world: 'state,
    {
        let id = store.get_component_id_assertive::<R>();
        // SAFETY: The scheduler MUST ensure that no system will mutably access this resource in parallel with this access
        unsafe {
            let res = if *store
                .resource_sendness
                .get(&id)
                .expect("Failed to find resource info")
            {
                store.send_resources.get_unsafe_ref::<R>(id)
            } else {
                store.non_send_resources.get_unsafe_ref::<R>(id)
            };
            std::mem::transmute(res.unwrap())
        }
    }

    fn create_initial_state(_store: &mut WorldContainer) -> Self::State {}

    fn on_entity_changed(
        _state: &mut Self::State,
        _store: &WorldContainer,
        _entity: Entity,
        _info: &EntityInfo,
    ) {
    }

    fn is_exclusive(world: &WorldContainer) -> bool {
        let id = world.get_component_id_assertive::<R>();
        !world
            .resource_sendness
            .get(&id)
            .expect("Failed to find component! Register it first")
    }
}

impl<'rworld, 'res, R: Resource + 'static> SystemParam for ResMut<'rworld, 'res, R> {
    type State = ();
    const IS_WORLD: bool = false;

    fn add_dependencies(
        store: &mut WorldContainer,
        components: &mut SparseSet<ComponentId, AccessMode>,
    ) {
        let id = store.get_component_id_assertive::<R>();
        components.insert(id, AccessMode::Read);
    }

    fn create<'world, 'state>(_data: &'state Self::State, store: &'world mut WorldContainer) -> Self
    where
        'world: 'state,
    {
        let id = store.get_component_id_assertive::<R>();
        // SAFETY: The scheduler MUST ensure that no other access is performed in parallel with this access
        unsafe {
            let res = if *store
                .resource_sendness
                .get(&id)
                .expect("Failed to find resource info")
            {
                store.send_resources.get_unsafe_mut_ref::<R>(id)
            } else {
                store.non_send_resources.get_unsafe_mut_ref::<R>(id)
            };
            std::mem::transmute(res.unwrap())
        }
    }

    fn create_initial_state(_store: &mut WorldContainer) -> Self::State {}

    fn on_entity_changed(
        _state: &mut Self::State,
        _store: &WorldContainer,
        _entity: Entity,
        _info: &EntityInfo,
    ) {
    }

    fn is_exclusive(world: &WorldContainer) -> bool {
        let id = world.get_component_id_assertive::<R>();
        !world
            .resource_sendness
            .get(&id)
            .expect("Failed to find component! Register it first")
    }
}

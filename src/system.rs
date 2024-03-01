use crate::{
    erased_data_vec::ErasedVec,
    query::{AccessMode, Query, QueryParam, QueryState},
    resources::{Res, ResMut, Resource, ResourceBase},
    sparse_set::SparseSet,
    ComponentId, Entity, EntityInfo, World,
};
use std::marker::PhantomData;

pub trait SystemParam: Sized {
    type State: Send + Sync + 'static;
    fn get_components(store: &mut World, components: &mut SparseSet<ComponentId, AccessMode>);
    fn create<'world, 'state>(data: &'state Self::State, store: &'world mut World) -> Self
    where
        'world: 'state;
    fn create_initial_state(store: &mut World) -> Self::State;
    fn on_entity_changed(state: &mut Self::State, store: &World, entity: Entity, info: &EntityInfo);
}
pub trait System: Send + Sync + 'static {
    fn init(&mut self, store: &mut World);
    fn run(&mut self, store: &mut World);
    fn on_entity_changed(&mut self, store: &World, entity: Entity, info: &EntityInfo);
}

impl<'qworld, 'qstate, A: QueryParam> SystemParam for Query<'qworld, 'qstate, A> {
    type State = QueryState;
    fn get_components(store: &mut World, components: &mut SparseSet<ComponentId, AccessMode>) {
        A::compute_component_set(store, components);
    }
    fn create<'world, 'state>(data: &'state Self::State, store: &'world mut World) -> Self
    where
        'world: 'state,
    {
        // SAFETY: We know that 'world: 'state, so we should be good to go
        unsafe { std::mem::transmute(Query::<'_, '_, A>::create_query(data, store.get_mut_ptr())) }
    }

    fn create_initial_state(store: &mut World) -> Self::State {
        let mut component_set = Default::default();
        Self::get_components(store, &mut component_set);

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
        store: &World,
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
        if system_archetype.includes(entity_archetype) {
            state.entities.insert(entity);
        } else {
            state.entities.remove(&entity);
        }
    }
}

pub struct SystemContainer<F, A> {
    _args: PhantomData<A>,
    fun: F,
    system_data: Vec<ErasedVec>,
}

impl<F, A> SystemContainer<F, A> {
    pub fn new(fun: F) -> Self {
        Self {
            _args: PhantomData,
            fun,
            system_data: vec![],
        }
    }
}

macro_rules! impl_system {
    ($($param:ident:$idx:expr)*) => {
        impl<$($param: SystemParam + Send + Sync + 'static,)* FUN: Fn($($param,)*) + Send + Sync + 'static> System
            for SystemContainer<FUN, ($($param,)*)>
        {
            fn init(&mut self, store: &mut World) {
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

            fn run(&mut self, store: &mut World) {
                (self.fun)($($param::create(unsafe { self.system_data[$idx].get::<$param::State>(0) }, store),)*);
            }

            fn on_entity_changed(&mut self, store: &World, entity: Entity, info: &EntityInfo) {
                {
                    $(
                        let state = unsafe { self.system_data[$idx].get_mut::<$param::State>(0) };
                        $param::on_entity_changed(state, store, entity, info);
                    )*
                }
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

impl<'rworld, 'res, R: Resource + Send + Sync + 'static> SystemParam for Res<'rworld, 'res, R> {
    type State = ();

    fn get_components(store: &mut World, components: &mut SparseSet<ComponentId, AccessMode>) {
        let id = store.get_component_id_assertive::<R>();
        components.insert(id, AccessMode::Read);
    }

    fn create<'world, 'state>(_data: &'state Self::State, store: &'world mut World) -> Self
    where
        'world: 'state,
    {
        let id = store.get_component_id_assertive::<R>();
        // SAFETY: The scheduler MUST ensure that no system will mutably access this resource in parallel with this access
        unsafe {
            let ptr = if R::SEND {
                store.send_resources.get_unsafe_ref::<R>(id)
            } else {
                store.non_send_resources.get_unsafe_ref::<R>(id)
            };
            std::mem::transmute(ptr.unwrap())
        }
    }

    fn create_initial_state(_store: &mut World) -> Self::State {}

    fn on_entity_changed(
        _state: &mut Self::State,
        _store: &World,
        _entity: Entity,
        _info: &EntityInfo,
    ) {
    }
}

impl<'rworld, 'res, R: Resource + 'static> SystemParam for ResMut<'rworld, 'res, R> {
    type State = ();

    fn get_components(store: &mut World, components: &mut SparseSet<ComponentId, AccessMode>) {
        let id = store.get_component_id_assertive::<R>();
        components.insert(id, AccessMode::Read);
    }

    fn create<'world, 'state>(_data: &'state Self::State, store: &'world mut World) -> Self
    where
        'world: 'state,
    {
        let id = store.get_component_id_assertive::<R>();
        // SAFETY: The scheduler MUST ensure that no system will mutably access this resource in parallel with this access
        unsafe {
            let ptr = if R::SEND {
                store.send_resources.get_unsafe_mut_ref::<R>(id)
            } else {
                store.non_send_resources.get_unsafe_mut_ref::<R>(id)
            };
            std::mem::transmute(ptr.unwrap())
        }
    }

    fn create_initial_state(_store: &mut World) -> Self::State {}

    fn on_entity_changed(
        _state: &mut Self::State,
        _store: &World,
        _entity: Entity,
        _info: &EntityInfo,
    ) {
    }
}

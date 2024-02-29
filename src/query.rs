use std::{collections::HashSet, marker::PhantomData};

use crate::{
    archetype::ArchetypeId, sparse_set::SparseSet, store::Store, ComponentId, Entity,
    UnsafeStorePtr,
};

pub enum AccessMode {
    Read,
    Write,
}

pub trait QueryParam {
    fn compute_component_set(
        store: &mut Store,
        component_set: &mut SparseSet<ComponentId, AccessMode>,
    );
    fn can_extract(store: &Store, entity: Entity) -> bool;
    unsafe fn extract(store: &UnsafeStorePtr, entity: Entity) -> Self;
}

pub struct Query<'world, 'state, A: QueryParam> {
    _ph: PhantomData<A>,
    state: &'state QueryState,
    world_ptr: UnsafeStorePtr<'world>,
}

#[derive(Default)]
pub struct QueryState {
    pub(crate) entities: HashSet<Entity>,
    pub(crate) query_archetype: ArchetypeId,
}

pub struct QueryIterator<'world, 'state, A: QueryParam> {
    _ph: PhantomData<A>,
    world_ptr: UnsafeStorePtr<'world>,
    entity_iterator: std::collections::hash_set::Iter<'state, Entity>,
}

impl<'world, 'state, A: QueryParam> Query<'world, 'state, A> {
    pub fn create_query(state: &'state QueryState, world_ptr: UnsafeStorePtr<'world>) -> Self {
        Self {
            _ph: PhantomData,
            state,
            world_ptr,
        }
    }

    pub fn iter(&self) -> QueryIterator<'world, 'state, A> {
        QueryIterator {
            _ph: PhantomData,
            world_ptr: self.world_ptr.clone(),
            entity_iterator: self.state.entities.iter(),
        }
    }
}

impl<'world, 'state, A: QueryParam> Iterator for QueryIterator<'world, 'state, A> {
    type Item = A;

    fn next(&mut self) -> Option<Self::Item> {
        self.entity_iterator
            .next()
            // SAFETY: The system scheduler must ensure that this unsafe call is safe
            .map(|e| unsafe { A::extract(&self.world_ptr, *e) })
    }
}

impl<A> QueryParam for &A
where
    A: 'static,
{
    unsafe fn extract(store: &UnsafeStorePtr, entity: Entity) -> Self {
        std::mem::transmute(store.get_component::<A>(entity).get())
    }

    fn can_extract(store: &Store, entity: Entity) -> bool {
        store.entity_has_component::<A>(entity)
    }

    fn compute_component_set(
        store: &mut Store,
        component_set: &mut SparseSet<ComponentId, AccessMode>,
    ) {
        let id = store.get_component_id_mut::<A>();
        if !component_set.insert(id, AccessMode::Read) {
            panic!("Query accesses twice the same component type! This is not allowed");
        }
    }
}

impl<A> QueryParam for &mut A
where
    A: 'static,
{
    unsafe fn extract(store: &UnsafeStorePtr, entity: Entity) -> Self {
        std::mem::transmute(store.get_component_mut::<A>(entity))
    }

    fn can_extract(store: &Store, entity: Entity) -> bool {
        store.entity_has_component::<A>(entity)
    }
    fn compute_component_set(
        store: &mut Store,
        component_set: &mut SparseSet<ComponentId, AccessMode>,
    ) {
        let id = store.get_component_id_mut::<A>();
        if !component_set.insert(id, AccessMode::Write) {
            panic!("Query accesses twice the same component type! This is not allowed");
        }
    }
}
macro_rules! impl_query_for_tuple {
    ($($t:ident)*) => {
        impl<$($t,)*> QueryParam for ($($t,)*)
        where
            $($t: QueryParam,)*
        {

            unsafe fn extract(store: &UnsafeStorePtr, entity: Entity) -> Self {
                ($($t::extract(store, entity),)*)
            }

            fn can_extract(store: &Store, entity: Entity) -> bool {
                $($t::can_extract(store, entity) &&)* true
            }
            fn compute_component_set(store: &mut Store, component_set: &mut SparseSet<ComponentId, AccessMode>) {
                $($t::compute_component_set(store, component_set);)*
            }
        }
    };
}

impl_query_for_tuple!(A);
impl_query_for_tuple!(A B);
impl_query_for_tuple!(A B C);
impl_query_for_tuple!(A B C D);
impl_query_for_tuple!(A B C D E);
impl_query_for_tuple!(A B C D E F);
impl_query_for_tuple!(A B C D E F G);
impl_query_for_tuple!(A B C D E F G H);
impl_query_for_tuple!(A B C D E F G H I);
impl_query_for_tuple!(A B C D E F G H I J);
impl_query_for_tuple!(A B C D E F G H I J K);
impl_query_for_tuple!(A B C D E F G H I J K L);
impl_query_for_tuple!(A B C D E F G H I J K L M);
impl_query_for_tuple!(A B C D E F G H I J K L M N);
impl_query_for_tuple!(A B C D E F G H I J K L M N O);
impl_query_for_tuple!(A B C D E F G H I J K L M N O P);
impl_query_for_tuple!(A B C D E F G H I J K L M N O P Q);
impl_query_for_tuple!(A B C D E F G H I J K L M N O P Q R);

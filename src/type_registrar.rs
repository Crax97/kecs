use std::{any::TypeId, collections::HashMap};

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default)]
pub struct UniqueTypeId(pub(crate) usize);

#[derive(Default, Debug)]
pub struct TypeRegistrar {
    counter: usize,
    registrations: HashMap<TypeId, UniqueTypeId>,
}

impl TypeRegistrar {
    pub fn get_registration<T: 'static>(&mut self) -> UniqueTypeId {
        *self
            .registrations
            .entry(TypeId::of::<T>())
            .or_insert_with(|| {
                let id = self.counter;
                self.counter += 1;
                UniqueTypeId(id)
            })
    }

    // The application will panic if T is not registered
    pub fn get<T: 'static>(&self) -> UniqueTypeId {
        *self
            .registrations
            .get(&TypeId::of::<T>())
            .expect("Type was not registered")
    }

    pub fn get_maybe<T: 'static>(&self) -> Option<UniqueTypeId> {
        self.registrations.get(&TypeId::of::<T>()).cloned()
    }

    pub(crate) fn get_from_type_id(&mut self, blob_ty_id: TypeId) -> UniqueTypeId {
        *self.registrations.entry(blob_ty_id).or_insert_with(|| {
            let id = self.counter;
            self.counter += 1;
            UniqueTypeId(id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests() {
        let mut registrar = TypeRegistrar::default();

        let id_i32 = registrar.get_registration::<i32>();
        assert!(id_i32 == registrar.get_registration::<i32>());

        let id_f32 = registrar.get_registration::<f32>();
        assert!(id_f32 == registrar.get_registration::<f32>());
        assert!(id_f32 != id_i32);

        let id_i32_2 = registrar.get::<i32>();
        assert!(id_i32_2 == id_i32);
        assert!(id_i32_2 != id_f32);
    }
}

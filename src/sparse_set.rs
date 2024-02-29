struct SparseKey<T> {
    index: usize,
    value: T,
}

pub trait SparseIndex: Default + From<usize> + Copy {
    fn index(&self) -> usize;
}

impl<I: Default + Copy + Clone + Into<usize> + From<usize>> SparseIndex for I {
    fn index(&self) -> usize {
        (*self).into()
    }
}

pub struct SparseSet<I, T> {
    dense: Vec<SparseKey<T>>,
    sparse: Vec<I>,
}

impl<T: Clone> Clone for SparseKey<T> {
    fn clone(&self) -> Self {
        Self {
            index: self.index,
            value: self.value.clone(),
        }
    }
}

impl<T: Clone, I: Clone> Clone for SparseSet<T, I> {
    fn clone(&self) -> Self {
        Self {
            dense: self.dense.clone(),
            sparse: self.sparse.clone(),
        }
    }
}

pub type SimpleSparseSet<I> = SparseSet<I, ()>;

impl<T, I: SparseIndex> Default for SparseSet<I, T> {
    fn default() -> Self {
        Self {
            dense: Default::default(),
            sparse: Default::default(),
        }
    }
}

impl<I: SparseIndex, T> SparseSet<I, T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.dense.len()
    }

    // returns true if index was not present in the set, false otherwise
    pub fn insert(&mut self, index: I, value: T) -> bool {
        let index = index.index();
        if let Some(key) = self.get_key(index) {
            self.dense[key].value = value;
            false
        } else {
            let len = self.len();
            self.dense.push(SparseKey { index, value });

            self.ensure_capacity_for(index);

            self.sparse[index] = len.into();
            true
        }
    }
    pub fn contains(&self, index: &I) -> bool {
        self.get(index).is_some()
    }

    pub fn get(&self, index: &I) -> Option<&T> {
        let index = index.index();
        self.get_key(index).map(|s| &self.dense[s].value)
    }

    pub fn get_mut(&mut self, index: I) -> Option<&mut T> {
        let index = index.index();
        self.get_key(index).map(|s| &mut self.dense[s].value)
    }

    pub fn remove(&mut self, index: I) -> bool {
        if self.get(&index).is_some() {
            let index = index.index();
            let n = self.dense.len() - 1;
            let index = self.sparse[index].index();
            let old_index = self.dense[n].index;

            self.dense.swap(index, n);
            self.sparse[old_index] = index.into();
            self.dense.pop();
            return true;
        }
        false
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.dense.iter().map(|d| (I::from(d.index), &d.value))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.dense.iter_mut().map(|d| &mut d.value)
    }

    pub fn clear(&mut self) {
        self.dense.clear();
    }
    fn get_key(&self, index: usize) -> Option<usize> {
        if index >= self.sparse.len() {
            return None;
        }
        let key_index = self.sparse[index].index();
        if key_index >= self.dense.len() {
            return None;
        }
        if index == self.dense[key_index].index {
            Some(key_index)
        } else {
            None
        }
    }

    fn ensure_capacity_for(&mut self, index: usize) {
        if self.sparse.len() <= index {
            self.sparse.resize(index + 1, I::default());
        }
    }

    pub fn get_or_insert(&mut self, index: I, fun: impl FnOnce() -> T) -> &mut T {
        if !self.contains(&index) {
            self.insert(index, fun());
        }

        self.get_mut(index).unwrap()
    }
}

impl<C: std::fmt::Debug> std::fmt::Debug for SparseKey<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SparseKey")
            .field("index", &self.index)
            .field("value", &self.value)
            .finish()
    }
}

impl<C: std::fmt::Debug, I: std::fmt::Debug> std::fmt::Debug for SparseSet<C, I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SparseSet")
            .field("dense", &self.dense)
            .field("sparse", &self.sparse)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const VALUE_MAX: usize = u16::MAX as usize;
    const ITERS_MAX: usize = 1000000;

    #[test]
    fn operations() {
        use rand::seq::SliceRandom;

        let mut sparse_set = SimpleSparseSet::new();
        let mut inserted_numbers: HashSet<usize> = Default::default();
        for _ in 0..rand::random::<usize>().max(1) % ITERS_MAX {
            let num: usize = rand::random::<usize>() % VALUE_MAX;
            if inserted_numbers.contains(&num) {
                continue;
            }
            inserted_numbers.insert(num);
            assert!(sparse_set.insert(num, ()));
        }

        for num in &inserted_numbers {
            assert!(sparse_set.contains(num));
        }

        let mut inserted = inserted_numbers.into_iter().collect::<Vec<_>>();
        inserted.shuffle(&mut rand::thread_rng());

        let mut removed_numbers = vec![];
        for _ in 0..rand::random::<usize>() % inserted.len() {
            let num = inserted.pop().unwrap();
            removed_numbers.push(num);
            assert!(sparse_set.remove(num));
        }

        for num in inserted {
            assert!(sparse_set.contains(&num));
        }

        for num in removed_numbers {
            assert!(!sparse_set.contains(&num));
        }

        sparse_set.clear();

        assert!(sparse_set.len() == 0);
    }
}

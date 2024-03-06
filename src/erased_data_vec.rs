use std::{alloc::Layout, marker::PhantomData, mem::MaybeUninit, num::NonZeroUsize, ptr::NonNull};

pub struct ErasedPtr<'a> {
    data: NonNull<u8>,
    ph_data: PhantomData<&'a u8>,
}
#[derive(Clone, Copy)]
pub struct UnsafePtr<'a, T: 'static>(pub(crate) *const T, pub(crate) PhantomData<&'a T>);

#[derive(Clone, Copy)]
pub struct UnsafeMutPtr<'a, T: 'static>(pub(crate) *mut T, pub(crate) PhantomData<&'a T>);
pub struct ErasedVec {
    pub(crate) layout: Layout,
    pub(crate) drop_fn: Option<unsafe fn(ErasedPtr<'_>)>,

    data: NonNull<u8>,

    len: usize,      // Specified in elements, not bytes
    capacity: usize, // Specified in elements, not bytes
}

pub unsafe fn make_drop_fn<T>(ptr: ErasedPtr<'_>) {
    let t = ptr.data.as_ptr().cast::<T>().read();
    std::mem::drop(t);
}

fn dangling_ptr_with_alignment(align: NonZeroUsize) -> NonNull<u8> {
    let align = align.get();
    unsafe { NonNull::new_unchecked(align as *mut u8) }
}

impl ErasedVec {
    pub unsafe fn new_typed<T>(with_drop_fn: bool, capacity: usize) -> Self {
        let layout = Layout::new::<T>();

        Self::new(layout, with_drop_fn.then_some(make_drop_fn::<T>), capacity)
    }

    pub unsafe fn new(
        layout: Layout,
        drop_fn: Option<unsafe fn(ErasedPtr<'_>)>,
        capacity: usize,
    ) -> Self {
        let data =
            dangling_ptr_with_alignment(NonZeroUsize::new(layout.align()).expect("Align was 0"));

        let mut me = Self {
            data,
            layout,
            drop_fn,
            len: 0,
            capacity: 0,
        };

        me.reserve_exact(capacity);

        me
    }

    pub unsafe fn push_back<T>(&mut self, value: T) -> usize {
        if self.len == self.capacity {
            self.grow_exact(1);
        }

        let index = self.len;

        if self.layout.size() == 0 {
            self.len += 1;
            return index;
        }

        let address = self.data.as_ptr().cast::<T>().add(self.len);
        address.cast::<T>().write(value);
        self.len += 1;
        index
    }

    pub unsafe fn remove<T>(&mut self, index: usize) -> T {
        assert!(index < self.len);

        let ptr = self.data.as_ptr().cast::<T>().add(index);
        let val = ptr.read();
        let remaining = self.len - index;
        std::ptr::copy(ptr.add(1), ptr, remaining - 1);
        self.len -= 1;

        val
    }

    pub fn clear(&mut self) {
        if let Some(drop_fn) = self.drop_fn {
            for i in 0..self.len {
                let current = unsafe { self.data.as_ptr().add(i * self.layout.size()) };
                unsafe {
                    drop_fn(ErasedPtr {
                        data: NonNull::new_unchecked(current),
                        ph_data: PhantomData,
                    })
                }
            }
        }
        self.len = 0;
    }

    pub unsafe fn get<T>(&self, index: usize) -> &T {
        assert!(index < self.len);
        let ptr = self.data.cast::<T>().as_ptr();
        ptr.add(index).as_ref().unwrap()
    }

    pub unsafe fn get_mut<T>(&mut self, index: usize) -> &mut T {
        assert!(index < self.len);
        let ptr = self.data.as_ptr().cast::<T>().add(index);
        ptr.as_mut().unwrap()
    }

    pub fn get_ptr(&self, index: usize) -> ErasedPtr<'_> {
        assert!(index < self.len);
        unsafe {
            let ptr = self.data.as_ptr().add(self.layout.size() * index);
            ErasedPtr {
                data: NonNull::new_unchecked(ptr),
                ph_data: PhantomData,
            }
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn reserve_exact(&mut self, new_elements: usize) {
        let available = self.capacity - self.len;

        if available < new_elements {
            let increment = new_elements - available;
            self.grow_exact(increment);
        }
    }

    pub fn grow_exact(&mut self, grow_elements: usize) {
        if grow_elements == 0 {
            return;
        }

        if self.layout.size() == 0 {
            self.capacity = usize::MAX;
            return;
        }

        let new_capacity = self.capacity + grow_elements;
        let new_layout = array_layout(self.layout, new_capacity);

        if self.layout.size() > 0 {
            if self.capacity == 0 {
                self.data = unsafe { NonNull::new_unchecked(std::alloc::alloc(new_layout)) };
            } else {
                self.data = unsafe {
                    NonNull::new_unchecked(std::alloc::realloc(
                        self.data.as_ptr(),
                        array_layout(self.layout, self.capacity),
                        new_layout.size(),
                    ))
                };
            }
        }
        self.capacity = new_capacity;
    }

    // Makes sure that there's space for new_len elements in the ErasedVec.
    // This function will never shrink the ErasedVec
    pub fn ensure_len(&mut self, new_len: usize) {
        if new_len > self.capacity {
            let difference = new_len - self.capacity;
            self.grow_exact(difference);
        }
        self.len = new_len;
    }

    /// This function will store the value at location index, without shifting all the other elements
    /// # SAFETY
    ///   The caller must ensure that T is the same type as the one stored in this ErasedVec,
    ///   that if the erased vec contains an item at the specified index, it was dropped,
    ///   and that there is enough space left in the ErasedVec for the type.
    ///   If index >= len, the programm will panic
    pub unsafe fn insert_at<T: 'static>(&self, index: usize, element: T) {
        assert!(index < self.len, "Not enough storage in the ErasedVec");
        self.get_ptr(index).data.as_ptr().cast::<T>().write(element)
    }

    /// # SAFETY
    ///   The caller must ensure that T is the same type as the one stored in this ErasedVec, that
    ///   index is a valid index for the vec (otherwise the program will will panic)
    ///   and that the value at the specified index hasn't been dropped yet.
    pub unsafe fn drop_at(&self, index: usize) {
        assert!(index < self.len, "Not enough storage in the ErasedVec");
        let ptr = self.get_ptr(index);
        if let Some(fun) = self.drop_fn {
            unsafe { fun(ptr) }
        }
    }

    /// # SAFETY
    /// The caller must ensure that the type of self and the data of `source` are the same
    /// and that the source must correctly deal with dropping the copied item
    pub fn copy_from(&self, dest_index: usize, source: &ErasedVec, source_index: usize) {
        assert!(dest_index < self.len);
        assert!(source_index < source.len);
        assert!(self.layout == source.layout, "layout mismatch!");

        unsafe {
            let dest_addr = self.data.as_ptr().add(dest_index * self.layout.size());
            let source_addr = source
                .data
                .as_ptr()
                .add(source_index * source.layout.size());
            dest_addr.copy_from(source_addr, self.layout.size());
        }
    }
}

impl Drop for ErasedVec {
    fn drop(&mut self) {
        if self.layout.size() == 0 {
            // No allocation was ever done
            return;
        }
        unsafe {
            std::alloc::dealloc(self.data.as_ptr(), array_layout(self.layout, self.capacity))
        };
    }
}

unsafe impl Send for ErasedVec {}
unsafe impl Sync for ErasedVec {}

impl<'a> ErasedPtr<'a> {
    pub unsafe fn cast<T: 'static>(self) -> UnsafePtr<'a, T> {
        UnsafePtr(self.data.cast::<T>().as_ptr().cast_const(), PhantomData)
    }

    pub unsafe fn cast_mut<T: 'static>(self) -> UnsafeMutPtr<'a, T> {
        UnsafeMutPtr(self.data.cast::<T>().as_ptr(), PhantomData)
    }
}

impl<'a, T> UnsafePtr<'a, T> {
    pub unsafe fn get(&self) -> &T {
        self.0.as_ref().unwrap()
    }

    pub unsafe fn into_ref(self) -> &'a T {
        self.0.as_ref().unwrap()
    }
}

impl<'a, T> UnsafeMutPtr<'a, T> {
    pub unsafe fn get(&self) -> &T {
        unsafe { self.0.as_ref().unwrap() }
    }
    pub unsafe fn get_mut(&mut self) -> &mut T {
        unsafe { self.0.as_mut().unwrap() }
    }

    pub unsafe fn into_ref(self) -> &'a T {
        self.0.as_ref().unwrap()
    }

    pub unsafe fn into_mut(self) -> &'a mut T {
        self.0.as_mut().unwrap()
    }
}

fn array_layout(layout: Layout, elements: usize) -> Layout {
    let padding = {
        let len = layout.size();
        let len_rounded = (len + layout.align() - 1) & !(layout.align() - 1);
        len_rounded - len
    };
    let padded_size = layout.size() + padding;
    Layout::from_size_align(padded_size * elements, layout.align()).unwrap()
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        ops::DerefMut,
        rc::Rc,
        sync::{Arc, RwLock},
    };

    use super::ErasedVec;

    struct TestStruct {
        foo: u32,
        string: String,
    }

    struct TestDrop {
        counter: Rc<RefCell<u32>>,
    }
    impl Drop for TestDrop {
        fn drop(&mut self) {
            let mut re = self.counter.borrow_mut();
            let re = re.deref_mut();
            *re += 1;
        }
    }
    struct ZeroSizedStruct;

    #[repr(align(32))]
    struct ZeroSizedStructWithWeirdAlignment;

    #[test]
    fn create_empty() {
        unsafe {
            let mut vec = ErasedVec::new_typed::<TestStruct>(true, 0);
            assert_eq!(vec.capacity(), 0);
            vec.reserve_exact(1);
            assert_eq!(vec.capacity(), 1);
        }
    }
    #[test]
    fn operations() {
        unsafe {
            let mut vec = ErasedVec::new_typed::<TestStruct>(true, 0);
            vec.push_back(TestStruct {
                foo: 0,
                string: "string0".to_owned(),
            });
            vec.push_back(TestStruct {
                foo: 1,
                string: "string1".to_owned(),
            });
            vec.push_back(TestStruct {
                foo: 2,
                string: "string2".to_owned(),
            });
            vec.push_back(TestStruct {
                foo: 3,
                string: "string3".to_owned(),
            });
            vec.push_back(TestStruct {
                foo: 4,
                string: "string4".to_owned(),
            });
            vec.push_back(TestStruct {
                foo: 5,
                string: "string5".to_owned(),
            });
            assert_eq!(vec.len, 6);

            for i in 0..6 {
                assert_eq!(vec.get::<TestStruct>(i).foo, i as u32);
                assert_eq!(
                    vec.get::<TestStruct>(i).string,
                    format!("string{i}").as_str()
                );
            }

            let removed = vec.remove::<TestStruct>(4);

            assert_eq!(removed.foo, 4);
            assert_eq!(removed.string, "string4");
            assert_eq!(vec.len(), 5);

            assert_eq!(vec.get::<TestStruct>(4).foo, 5);
            assert_eq!(vec.get::<TestStruct>(4).string, "string5");

            for i in 0..1000 {
                vec.push_back(TestStruct {
                    foo: i,
                    string: "helloworldthisisalongstringandhopefullyitshouldnotbreak".to_owned(),
                });
            }

            let modified = vec.get_mut::<TestStruct>(300);
            modified.foo = 42;

            let taken_modified = vec.remove::<TestStruct>(300);
            assert_eq!(taken_modified.foo, 42);

            vec.clear();

            assert!(vec.len() == 0);
        }
    }

    #[test]
    fn drop_test() {
        let counter = Rc::new(RefCell::new(0));
        unsafe {
            let mut vec = ErasedVec::new_typed::<TestDrop>(true, 0);
            vec.push_back(TestDrop {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop {
                counter: counter.clone(),
            });
            for i in 0..vec.len() {
                vec.drop_at(i);
            }
            let c: u32 = *counter.borrow();
            assert_eq!(c, 6);

            let mut vec = ErasedVec::new_typed::<TestStruct>(true, 1);
            vec.push_back(TestStruct {
                foo: 42,
                string: "Hello World".to_string(),
            });
            vec.drop_at(0);
        }
    }

    #[test]
    fn drop_test_2() {
        let counter = Arc::new(RwLock::new(0));
        struct TestDrop2 {
            counter: Arc<RwLock<usize>>,
        }
        impl Drop for TestDrop2 {
            fn drop(&mut self) {
                let mut re = self.counter.write().unwrap();
                let re = re.deref_mut();
                *re += 1;
            }
        }
        unsafe {
            let mut vec = ErasedVec::new_typed::<TestDrop2>(true, 0);
            vec.push_back(TestDrop2 {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop2 {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop2 {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop2 {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop2 {
                counter: counter.clone(),
            });
            vec.push_back(TestDrop2 {
                counter: counter.clone(),
            });
            for i in 0..vec.len() {
                vec.drop_at(i);
            }
            let c: usize = *counter.read().unwrap();
            assert_eq!(c, 6);

            let mut vec = ErasedVec::new_typed::<TestStruct>(true, 1);
            vec.push_back(TestStruct {
                foo: 42,
                string: "Hello World".to_string(),
            });
            vec.drop_at(0);
        }
    }
    #[test]
    fn zero_sized() {
        unsafe {
            let mut vec = ErasedVec::new_typed::<ZeroSizedStruct>(true, 0);
            vec.reserve_exact(1000);
            vec.push_back(ZeroSizedStruct);
            vec.push_back(ZeroSizedStruct);
            vec.push_back(ZeroSizedStruct);
            vec.push_back(ZeroSizedStruct);
            vec.push_back(ZeroSizedStruct);
            vec.push_back(ZeroSizedStruct);

            assert_eq!(vec.len(), 6);

            vec.clear();

            assert_eq!(vec.len(), 0);
            drop(vec)
        }
    }
    #[test]
    fn aligned_data() {
        unsafe {
            let mut vec = ErasedVec::new_typed::<ZeroSizedStructWithWeirdAlignment>(true, 0);
            vec.reserve_exact(1000);
            vec.push_back(ZeroSizedStructWithWeirdAlignment);
            vec.push_back(ZeroSizedStructWithWeirdAlignment);
            vec.push_back(ZeroSizedStructWithWeirdAlignment);
            vec.push_back(ZeroSizedStructWithWeirdAlignment);
            vec.push_back(ZeroSizedStructWithWeirdAlignment);
            vec.push_back(ZeroSizedStructWithWeirdAlignment);

            assert_eq!(vec.len(), 6);

            vec.clear();

            assert_eq!(vec.len(), 0);
            drop(vec)
        }
    }

    #[test]
    fn copy_vec() {
        struct TestStruct {
            number: u32,
            string: String,
        }

        unsafe {
            let mut vec1 = ErasedVec::new_typed::<TestStruct>(true, 1);
            let mut vec2 = ErasedVec::new_typed::<TestStruct>(true, 1);

            const STR: &str = "Hello World!";
            vec1.push_back(TestStruct {
                number: 42,
                string: STR.to_owned(),
            });

            vec2.ensure_len(1);
            vec2.copy_from(0, &vec1, 0);

            assert_eq!(vec2.get::<TestStruct>(0).number, 42);
            assert_eq!(vec2.get::<TestStruct>(0).string, STR);

            vec2.drop_at(0);
        }
    }
}

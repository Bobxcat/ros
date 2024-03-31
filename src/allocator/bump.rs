use core::{alloc::GlobalAlloc, ops::DerefMut, ptr};

use spin::Mutex;

use super::align_up;

struct BumpAllocInner {
    heap_start: usize,
    heap_end: usize,
    next: usize,
    allocations: usize,
}

impl BumpAllocInner {
    pub const fn new() -> Self {
        Self {
            heap_start: 0,
            heap_end: 0,
            next: 0,
            allocations: 0,
        }
    }
    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.heap_start = heap_start;
        self.heap_end = heap_start + heap_size;
        self.next = heap_start;
    }
}

pub struct BumpAlloc {
    inner: Mutex<BumpAllocInner>,
}

impl BumpAlloc {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(BumpAllocInner::new()),
        }
    }
    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        unsafe { self.inner.lock().init(heap_start, heap_size) }
    }
    fn lock<'a>(&'a self) -> impl DerefMut<Target = BumpAllocInner> + 'a {
        self.inner.lock()
    }
}

unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let mut inner = self.lock();

        let alloc_start = align_up(inner.next, layout.align());
        let alloc_end = match alloc_start.checked_add(layout.size()) {
            Some(end) => end,
            None => return ptr::null_mut(),
        };

        if alloc_end > inner.heap_end {
            ptr::null_mut()
        } else {
            inner.next = alloc_end;
            inner.allocations += 1;
            alloc_start as *mut _
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        let mut inner = self.lock();

        inner.allocations -= 1;
        if inner.allocations == 0 {
            inner.next = inner.heap_start;
        }
    }
}

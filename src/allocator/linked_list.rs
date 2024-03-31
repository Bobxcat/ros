use core::{
    alloc::{GlobalAlloc, Layout},
    mem,
    ops::DerefMut,
    ptr,
};

use spin::Mutex;

use crate::allocator::align_up;

struct Node {
    size: usize,
    next: Option<*mut Node>,
}

impl Node {
    const fn new(size: usize) -> Self {
        Self { size, next: None }
    }

    fn start_addr(&self) -> usize {
        self as *const Self as usize
    }

    fn end_addr(&self) -> usize {
        self.start_addr() + self.size
    }
}

struct LinkedListAllocInner {
    head: Node,
}

impl LinkedListAllocInner {
    pub const fn new() -> Self {
        LinkedListAllocInner { head: Node::new(0) }
    }
    /// Initialize the allocator with the given heap bounds.
    ///
    /// This function is unsafe because the caller must guarantee that the given
    /// heap bounds are valid and that the heap is unused. This method must be
    /// called only once.
    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        unsafe { self.add_free_region(heap_start, heap_size) }
    }

    /// Adds the given memory region to the front of the list.
    unsafe fn add_free_region(&mut self, addr: usize, size: usize) {
        // ensure that the freed region is capable of holding Node
        assert_eq!(align_up(addr, mem::align_of::<Node>()), addr);
        assert!(size >= mem::size_of::<Node>());

        // create a new list node and append it at the start of the list
        let mut node = Node::new(size);
        node.next = self.head.next.take();

        let node_ptr = addr as *mut Node;
        unsafe { node_ptr.write(node) }
        self.head.next = Some(node_ptr)
    }

    /// Looks for a free region with the given size and alignment and removes
    /// it from the list.
    ///
    /// Returns a tuple of the list node and the start address of the allocation.
    fn find_region(&mut self, size: usize, align: usize) -> Option<(*mut Node, usize)> {
        // reference to current list node, updated for each iteration
        let mut current = &mut self.head;
        // look for a large enough memory region in linked list
        while let Some(region) = current.next {
            let region = unsafe { &mut *region };
            if let Ok(alloc_start) = Self::alloc_from_region(&region, size, align) {
                // region suitable for allocation -> remove node from list
                let new_next = region.next.take();
                let ret = Some((current.next.take().unwrap(), alloc_start));
                current.next = new_next;
                return ret;
            } else {
                // region not suitable -> continue with next region
                current = region;
            }
        }

        // no suitable region found
        None
    }

    /// Try to use the given region for an allocation with given size and
    /// alignment.
    ///
    /// Returns the allocation start address on success.
    fn alloc_from_region(region: &Node, size: usize, align: usize) -> Result<usize, ()> {
        let alloc_start = align_up(region.start_addr(), align);
        let alloc_end = alloc_start.checked_add(size).ok_or(())?;

        if alloc_end > region.end_addr() {
            // region too small
            return Err(());
        }

        let excess_size = region.end_addr() - alloc_end;
        if excess_size > 0 && excess_size < mem::size_of::<Node>() {
            // rest of region too small to hold a ListNode (required because the
            // allocation splits the region in a used and a free part)
            return Err(());
        }

        // region suitable for allocation
        Ok(alloc_start)
    }

    /// Adjust the given layout so that the resulting allocated memory
    /// region is also capable of storing a `ListNode`.
    ///
    /// Returns the adjusted size and alignment as a (size, align) tuple.
    fn size_align(layout: Layout) -> (usize, usize) {
        let layout = layout
            .align_to(mem::align_of::<Node>())
            .expect("adjusting alignment failed")
            .pad_to_align();
        let size = layout.size().max(mem::size_of::<Node>());
        (size, layout.align())
    }
}

pub struct LinkedListAlloc {
    inner: Mutex<LinkedListAllocInner>,
}

impl LinkedListAlloc {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(LinkedListAllocInner::new()),
        }
    }

    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        unsafe { self.lock().init(heap_start, heap_size) }
    }

    fn lock<'a>(&'a self) -> impl DerefMut<Target = LinkedListAllocInner> + 'a {
        self.inner.lock()
    }
}

unsafe impl GlobalAlloc for LinkedListAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let (size, align) = LinkedListAllocInner::size_align(layout);
        let mut s = self.lock();

        if let Some((region, alloc_start)) = s.find_region(size, align) {
            let alloc_end = alloc_start.checked_add(size).expect("overflow");
            let excess_size = unsafe { &*region }.end_addr() - alloc_end;
            if excess_size > 0 {
                unsafe { s.add_free_region(alloc_end, excess_size) };
            }
            alloc_start as *mut u8
        } else {
            ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let (size, _) = LinkedListAllocInner::size_align(layout);
        unsafe { self.lock().add_free_region(ptr as usize, size) }
    }
}

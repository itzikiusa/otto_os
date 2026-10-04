//! Allocation volume, not timing or allocator address reuse, pins amortization.
use otto_pty::ring::RingBuffer;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountAlloc;
thread_local! { static BYTES: Cell<Option<usize>> = const { Cell::new(None) }; }
fn charge(bytes: usize) {
    BYTES.with(|n| {
        if let Some(old) = n.get() {
            n.set(Some(old + bytes));
        }
    });
}
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        charge(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        charge(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOC: CountAlloc = CountAlloc;

#[test]
fn newline_free_stream_allocations_scale_with_input_not_retained_suffix() {
    let mut ring = RingBuffer::new(100, 64 * 1024);
    ring.push(&vec![b'a'; 64 * 1024]);
    let chunk = [b'b'; 1024];
    BYTES.with(|n| n.set(Some(0)));
    for _ in 0..1024 {
        ring.push(&chunk);
    }
    let allocated = BYTES.with(|n| n.replace(None).unwrap());
    eprintln!("newline-free ring: input=1048576 retained=65536 allocated={allocated} bytes");
    assert!(
        allocated < 8 * 1024 * 1024,
        "1 MiB output allocated {allocated} bytes"
    );
    assert_eq!(ring.tail(1), vec![b'b'; 64 * 1024]);
}

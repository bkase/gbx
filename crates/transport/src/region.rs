//! Memory mapping helpers for the transport rings.
//!
//! The transport spec relies on fixed-size SharedArrayBuffer (web) or mmap
//! regions (native). This module offers a cross-platform abstraction that
//! allocates aligned, contiguous memory while keeping the unsafe surface
//! tightly encapsulated.

use crate::{TransportError, TransportResult};
use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::marker::PhantomData;
use std::mem;
use std::ptr::NonNull;

#[cfg(not(target_arch = "wasm32"))]
use std::ptr;

#[cfg(not(target_arch = "wasm32"))]
type NativeMap = memmap2::MmapMut;

#[derive(Debug)]
enum Backing {
    #[cfg(not(target_arch = "wasm32"))]
    Native(NativeMap),
    Owned {
        ptr: NonNull<u8>,
        layout: Layout,
    },
}

impl Backing {
    fn as_mut_ptr(&mut self) -> *mut u8 {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Backing::Native(map) => map.as_mut_ptr(),
            Backing::Owned { ptr, .. } => ptr.as_ptr(),
        }
    }

    fn as_ptr(&self) -> *const u8 {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Backing::Native(map) => map.as_ptr(),
            Backing::Owned { ptr, .. } => ptr.as_ptr(),
        }
    }
}

/// Backing memory for message rings and slot pools.
///
/// Native targets prefer anonymous `mmap` regions (page aligned). When that is
/// not possible—or on WebAssembly—we fall back to heap allocations while
/// honoring the requested alignment.
/// Marker for a [`SharedRegion`] whose bytes are known to be initialised.
#[derive(Debug)]
pub enum Zeroed {}

/// Marker for a [`SharedRegion`] that may contain uninitialised bytes.
#[derive(Debug)]
pub enum Uninit {}

/// Backing memory for message rings and slot pools parameterised by initialisation state.
#[derive(Debug)]
pub struct SharedRegion<State> {
    len: usize,
    alignment: usize,
    backing: Backing,
    _marker: PhantomData<State>,
}

#[derive(Clone, Copy)]
enum InitKind {
    Zeroed,
    Uninitialized,
}

impl InitKind {
    #[cfg(not(target_arch = "wasm32"))]
    fn is_zeroed(self) -> bool {
        matches!(self, InitKind::Zeroed)
    }
}

fn allocate_backing(len: usize, alignment: usize, init: InitKind) -> TransportResult<Backing> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(TransportError::AllocationFailed {
            size: len,
            alignment,
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(backing) = mmap_backing(len, alignment, init)? {
            return Ok(backing);
        }
    }

    heap_backing(len, alignment, init)
}

fn heap_backing(len: usize, alignment: usize, init: InitKind) -> TransportResult<Backing> {
    let layout =
        Layout::from_size_align(len, alignment).map_err(|_| TransportError::AllocationFailed {
            size: len,
            alignment,
        })?;

    // SAFETY: `alloc` and `alloc_zeroed` return either a valid pointer for `layout` or null on
    // allocation failure; we check for null immediately afterwards.
    let ptr = unsafe {
        match init {
            InitKind::Zeroed => alloc_zeroed(layout),
            InitKind::Uninitialized => alloc(layout),
        }
    };

    let ptr = NonNull::new(ptr).ok_or(TransportError::AllocationFailed {
        size: len,
        alignment,
    })?;

    Ok(Backing::Owned { ptr, layout })
}

#[cfg(not(target_arch = "wasm32"))]
fn mmap_backing(
    len: usize,
    alignment: usize,
    init: InitKind,
) -> Result<Option<Backing>, TransportError> {
    let mut map = memmap2::MmapOptions::new()
        .len(len)
        .map_anon()
        .map_err(|_| TransportError::AllocationFailed {
            size: len,
            alignment,
        })?;

    let ptr = map.as_mut_ptr();
    if !(ptr as usize).is_multiple_of(alignment) {
        return Ok(None);
    }

    if init.is_zeroed() {
        // SAFETY: the anonymous mapping exposes `len` bytes that can be zeroed here.
        unsafe { ptr::write_bytes(ptr, 0, len) };
    }

    Ok(Some(Backing::Native(map)))
}

impl<State> SharedRegion<State> {
    fn from_backing(len: usize, alignment: usize, backing: Backing) -> Self {
        Self {
            len,
            alignment,
            backing,
            _marker: PhantomData,
        }
    }

    fn into_state<Next>(self) -> SharedRegion<Next> {
        // SAFETY: `SharedRegion<State>` and `SharedRegion<Next>` share identical layout and drop
        // semantics because the marker type does not affect stored data.
        unsafe { mem::transmute(self) }
    }

    /// Total number of bytes managed by this region.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true when the region has zero length.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the alignment the region was allocated with.
    pub fn alignment(&self) -> usize {
        self.alignment
    }

    /// Borrow the region as a const pointer.
    pub fn as_ptr(&self) -> *const u8 {
        self.backing.as_ptr()
    }

    /// Borrow the region as a mut pointer.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.backing.as_mut_ptr()
    }

    /// View the full region as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `SharedRegion` owns an allocation of `len` bytes, so the derived pointer is
        // in-bounds and uniquely borrowed for the lifetime of `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.as_mut_ptr(), self.len) }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn wasm_region(&self) -> crate::wasm::Region {
        use core::convert::TryFrom;

        crate::wasm::Region {
            offset: self.as_ptr() as u32,
            length: u32::try_from(self.len)
                .expect("shared region length must fit into 32 bits on wasm32"),
        }
    }

    fn assert_view_bounds<T>(&self, offset_bytes: usize, len: usize) {
        let elem_size = mem::size_of::<T>();
        if elem_size == 0 {
            return;
        }
        let span_bytes = len.checked_mul(elem_size).expect("slice length overflow");
        let end = offset_bytes
            .checked_add(span_bytes)
            .expect("slice bounds overflow");
        assert!(
            end <= self.len,
            "slice of {} bytes exceeds region length {}",
            end,
            self.len
        );

        let base = self.as_ptr() as usize + offset_bytes;
        let align = mem::align_of::<T>();
        assert!(
            base.is_multiple_of(align),
            "region offset {offset_bytes} misaligned for type with alignment {align}"
        );
    }
}

impl SharedRegion<Zeroed> {
    /// Allocates a new region of `len` bytes aligned to `alignment`, fully zeroed.
    pub fn new_aligned_zeroed(len: usize, alignment: usize) -> TransportResult<Self> {
        let backing = allocate_backing(len, alignment, InitKind::Zeroed)?;
        Ok(Self::from_backing(len, alignment, backing))
    }

    /// View the full region as an immutable slice.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: The region owns `len` initialised bytes and exposes them immutably.
        unsafe { std::slice::from_raw_parts(self.as_ptr(), self.len) }
    }

    /// Reinterpret the prefix of the region as a reference to `T`.
    ///
    /// `T` must be plain-old-data (no drop glue) and the bytes must already be initialised
    /// before the returned reference is read.
    pub(crate) fn prefix<T>(&self) -> &T {
        debug_assert!(
            !mem::needs_drop::<T>(),
            "header types must be plain-old-data without drop glue"
        );
        self.assert_view_bounds::<T>(0, 1);
        // SAFETY: Bounds and alignment checked above; caller guarantees the bytes are initialised
        // and represent a `T` value.
        unsafe { &*(self.as_ptr() as *const T) }
    }

    /// Reinterpret the prefix of the region as a mutable reference to `T`.
    ///
    /// `T` must be plain-old-data (no drop glue). Callers must fully initialise the written
    /// bytes before materialising any immutable reference to them.
    pub(crate) fn prefix_mut<T>(&mut self) -> &mut T {
        debug_assert!(
            !mem::needs_drop::<T>(),
            "header types must be plain-old-data without drop glue"
        );
        self.assert_view_bounds::<T>(0, 1);
        // SAFETY: Bounds and alignment checked above; the mutable borrow ensures exclusive access so
        // callers can initialise the bytes as a `T`.
        unsafe { &mut *(self.as_mut_ptr() as *mut T) }
    }

    /// Returns a typed slice view into the region starting at `offset_bytes`.
    pub(crate) fn slice<T>(&self, offset_bytes: usize, len: usize) -> &[T] {
        self.assert_view_bounds::<T>(offset_bytes, len);
        // SAFETY: `assert_view_bounds` guarantees `offset_bytes + len * size_of::<T>()` lies within
        // the allocation and that the resulting pointer satisfies alignment.
        let ptr = unsafe { self.as_ptr().add(offset_bytes) } as *const T;
        // SAFETY: Pointer and length are validated above; initialisation is the caller's
        // responsibility where applicable.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    /// Returns a mutable typed slice view into the region starting at `offset_bytes`.
    pub(crate) fn slice_mut<T>(&mut self, offset_bytes: usize, len: usize) -> &mut [T] {
        self.assert_view_bounds::<T>(offset_bytes, len);
        // SAFETY: `assert_view_bounds` establishes the offset fits in the allocation and obeys
        // alignment; the mutable borrow ensures unique access for the produced slice.
        let ptr = unsafe { self.as_mut_ptr().add(offset_bytes) } as *mut T;
        // SAFETY: Pointer and length have been checked above; caller initialises the region before
        // reading from it.
        unsafe { std::slice::from_raw_parts_mut(ptr, len) }
    }
}

impl SharedRegion<Uninit> {
    /// Allocates a new region of `len` bytes aligned to `alignment`, leaving bytes uninitialised.
    pub fn new_aligned_uninit(len: usize, alignment: usize) -> TransportResult<Self> {
        let backing = allocate_backing(len, alignment, InitKind::Uninitialized)?;
        Ok(Self::from_backing(len, alignment, backing))
    }

    /// Marks the region as initialised after the caller has written valid bytes to it.
    pub fn assume_init(self) -> SharedRegion<Zeroed> {
        self.into_state()
    }
}

impl<State> Drop for SharedRegion<State> {
    fn drop(&mut self) {
        match &self.backing {
            Backing::Owned { ptr, layout } => {
                // SAFETY: `ptr`/`layout` originate from `alloc` in `heap_backing`; they stay valid
                // until this drop runs, so deallocating here releases the allocation once.
                unsafe {
                    dealloc(ptr.as_ptr(), *layout);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backing::Native(_) => {}
        }
    }
}

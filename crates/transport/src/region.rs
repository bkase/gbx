//! Memory mapping helpers for the transport rings.
//!
//! The transport spec relies on fixed-size SharedArrayBuffer (web) or mmap
//! regions (native). This module offers a cross-platform abstraction that
//! allocates aligned, contiguous memory while keeping the unsafe surface
//! tightly encapsulated.

use crate::{TransportError, TransportResult};
use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::mem;
use std::ptr::{self, NonNull};

/// Specifies how memory in a [`SharedRegion`] should be initialised.
#[derive(Clone, Copy, Debug)]
pub enum RegionInit {
    /// Zero the entire region after allocation.
    Zeroed,
    /// Leave the region uninitialised.
    Uninitialized,
}

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
#[derive(Debug)]
pub struct SharedRegion {
    len: usize,
    alignment: usize,
    backing: Backing,
}

impl SharedRegion {
    /// Allocates a new region of `len` bytes aligned to `alignment`.
    ///
    /// On native builds we first try to satisfy the request via `mmap`. If the
    /// returned pointer is not suitably aligned, we transparently fall back to
    /// the heap implementation.
    pub fn new_aligned(len: usize, alignment: usize, init: RegionInit) -> TransportResult<Self> {
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(TransportError::AllocationFailed {
                size: len,
                alignment,
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(backing) = Self::mmap_backed(len, alignment, init)? {
                return Ok(Self {
                    len,
                    alignment,
                    backing,
                });
            }
        }

        Self::heap_backed(len, alignment, init)
    }

    fn heap_backed(len: usize, alignment: usize, init: RegionInit) -> TransportResult<Self> {
        let layout = Layout::from_size_align(len, alignment).map_err(|_| {
            TransportError::AllocationFailed {
                size: len,
                alignment,
            }
        })?;

        let ptr = unsafe {
            match init {
                RegionInit::Zeroed => alloc_zeroed(layout),
                RegionInit::Uninitialized => alloc(layout),
            }
        };

        let ptr = NonNull::new(ptr).ok_or(TransportError::AllocationFailed {
            size: len,
            alignment,
        })?;
        Ok(Self {
            len,
            alignment,
            backing: Backing::Owned { ptr, layout },
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn mmap_backed(
        len: usize,
        alignment: usize,
        init: RegionInit,
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

        if matches!(init, RegionInit::Zeroed) {
            unsafe {
                // SAFETY: the anonymous mapping exposes `len` bytes that can be zeroed here.
                ptr::write_bytes(ptr, 0, len)
            };
        }

        Ok(Some(Backing::Native(map)))
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

    /// View the full region as an immutable slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.as_ptr(), self.len) }
    }

    /// View the full region as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.as_mut_ptr(), self.len) }
    }

    /// Reinterpret the prefix of the region as a reference to `T`.
    pub(crate) fn prefix<T>(&self) -> &T {
        self.assert_view_bounds::<T>(0, 1);
        unsafe { &*(self.as_ptr() as *const T) }
    }

    /// Reinterpret the prefix of the region as a mutable reference to `T`.
    pub(crate) fn prefix_mut<T>(&mut self) -> &mut T {
        self.assert_view_bounds::<T>(0, 1);
        unsafe { &mut *(self.as_mut_ptr() as *mut T) }
    }

    /// Returns a typed slice view into the region starting at `offset_bytes`.
    pub(crate) fn slice<T>(&self, offset_bytes: usize, len: usize) -> &[T] {
        self.assert_view_bounds::<T>(offset_bytes, len);
        let ptr = unsafe { self.as_ptr().add(offset_bytes) } as *const T;
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    /// Returns a mutable typed slice view into the region starting at `offset_bytes`.
    pub(crate) fn slice_mut<T>(&mut self, offset_bytes: usize, len: usize) -> &mut [T] {
        self.assert_view_bounds::<T>(offset_bytes, len);
        let ptr = unsafe { self.as_mut_ptr().add(offset_bytes) } as *mut T;
        unsafe { std::slice::from_raw_parts_mut(ptr, len) }
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
            base % align == 0,
            "region offset {offset_bytes} misaligned for type with alignment {align}"
        );
    }
}

impl Drop for SharedRegion {
    fn drop(&mut self) {
        if let Backing::Owned { ptr, layout } = &self.backing {
            unsafe {
                dealloc(ptr.as_ptr(), *layout);
            }
        }
    }
}

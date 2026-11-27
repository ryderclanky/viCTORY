//! Host copies of title memory placed in shared heaps.
//!
//! After the texture heaps (shadowdog00/vi-pc-combo, and `crate::heaps`):
//! one committed resource per buffer is a driver allocation of its own, and
//! on a 16 GB machine those allocations are what is left once textures
//! share heaps. Windows and the small title buffers are write-back memory
//! the CPU fills and shaders read. Buffers from 64 KiB to 16 MiB are cut
//! from 64 MiB heaps of that memory. A smaller buffer stays committed (a
//! placed one would take 64 KiB), and a larger one stays committed so it
//! cannot pin a heap. A range comes back only once the resource's last
//! reference is the heap's own note of it.
//!
//! `VICTORY_BUFFER_HEAPS=1` places them. Unset, each buffer stays committed:
//! the heaps commit 64 MiB whole, and that raised private RAM on a 16 GB
//! machine.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

use crate::laptop::RangePool;

/// 64 KiB, the alignment a placed buffer takes.
const PLAIN: u64 = 64 << 10;
/// One heap. The free tail of the last one is at most this, and an empty
/// heap past the first is given back.
const HEAP_BYTES: u64 = 64 << 20;
/// Free bytes kept when an empty heap is given back. One empty heap stays
/// so the next buffer does not pay for creating it.
const HEAP_KEPT: u64 = HEAP_BYTES;

struct BufferHeap {
    heap: ID3D12Heap,
    ranges: RangePool,
    size: u64,
}

// SAFETY: a free-threaded D3D12 heap.
unsafe impl Send for BufferHeap {}

/// A range of a heap. Dropping it gives the bytes back.
struct Cut {
    heap: usize,
    offset: u64,
    len: u64,
}

// SAFETY: an index and a length; the heap itself stays in the pool.
unsafe impl Send for Cut {}

impl Drop for Cut {
    fn drop(&mut self) {
        give_back(self.heap, self.offset, self.len);
    }
}

/// The placed buffer and the range that keeps its memory. The resource is
/// dropped first, then the range, so the bytes are not cut again while the
/// buffer still names them.
struct Held {
    resource: ID3D12Resource,
    #[allow(dead_code)]
    cut: Cut,
}

// SAFETY: a free-threaded D3D12 resource and a range.
unsafe impl Send for Held {}

static HEAPS: Mutex<Vec<Option<BufferHeap>>> = Mutex::new(Vec::new());
static HELD: Mutex<Vec<Held>> = Mutex::new(Vec::new());
/// Bytes of the heaps that no buffer is placed in.
static FREE: AtomicU64 = AtomicU64::new(0);

/// Whether new shared buffers are placed in heaps.
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = crate::laptop::buffer_heaps_on(
            std::env::var("VICTORY_BUFFER_HEAPS").ok().as_deref(),
        );
        if on {
            tracing::info!(
                "host copies of title memory share 64 MiB heaps (VICTORY_BUFFER_HEAPS; =0: one committed buffer each)"
            );
        }
        on
    })
}

/// Lets go of placed buffers nothing else holds. Called from the idle sweep
/// and before a new range is cut.
pub(crate) fn reclaim() {
    let gone = {
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        let mut gone = Vec::new();
        let mut i = 0;
        while i < held.len() {
            if crate::host::references(&held[i].resource) == 1 {
                gone.push(held.swap_remove(i));
            } else {
                i += 1;
            }
        }
        gone
    };
    drop(gone);
}

/// A buffer of `size` bytes in a shared heap, mapped, or `None` when heaps
/// are off, the size is not one a heap cuts, or the heap could not be made.
/// The caller holds the resource. This module keeps another reference until
/// that one is the last, then gives the range back.
pub(crate) fn place(host: &crate::host::Host, size: u64) -> Option<(ID3D12Resource, *mut u8)> {
    if !enabled() {
        return None;
    }
    let len = crate::laptop::buffer_heap_len(size)?;
    reclaim();
    let (heap, cut) = cut_range(host, len)?;
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Alignment: PLAIN,
        Width: len,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
        ..Default::default()
    };
    let mut buffer: Option<ID3D12Resource> = None;
    // SAFETY: `cut` is this buffer's alone until the held note is dropped,
    // which is after the caller's reference.
    let made = unsafe {
        host.device.CreatePlacedResource(
            &heap,
            cut.offset,
            &desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut buffer,
        )
    };
    let buffer = match made.and_then(|()| buffer.ok_or_else(windows::core::Error::empty)) {
        Ok(buffer) => buffer,
        Err(e) => {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                tracing::warn!(
                    error = %e,
                    "no placed host buffer; a committed one instead"
                )
            });
            return None;
        }
    };
    let mut data = std::ptr::null_mut();
    // SAFETY: a live buffer in a CPU-visible heap. The mapping lasts as
    // long as the buffer, which the caller holds while it reads the pointer.
    let mapped = unsafe { buffer.Map(0, None, Some(&mut data)) };
    if let Err(e) = mapped {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            tracing::warn!(error = %e, "a placed host buffer could not be mapped; a committed one instead")
        });
        return None;
    }
    HELD.lock().unwrap_or_else(|e| e.into_inner()).push(Held {
        resource: buffer.clone(),
        cut,
    });
    Some((buffer, data.cast()))
}

fn cut_range(host: &crate::host::Host, len: u64) -> Option<(ID3D12Heap, Cut)> {
    let mut heaps = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    let found = heaps.iter_mut().enumerate().find_map(|(i, slot)| {
        let heap = slot.as_mut()?;
        heap.ranges.take(len, PLAIN).map(|offset| (i, offset))
    });
    let (index, offset) = match found {
        Some(found) => found,
        None => {
            // A heap is system memory, committed whole. Skip it when the
            // commit left would not cover the heap and a margin, and the
            // caller makes one committed buffer instead.
            if crate::tuning::commit_free().is_some_and(|free| free < HEAP_BYTES + (256 << 20)) {
                return None;
            }
            let heap_desc = D3D12_HEAP_DESC {
                SizeInBytes: HEAP_BYTES,
                Properties: D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_CUSTOM,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_L0,
                    ..Default::default()
                },
                Alignment: 0,
                Flags: D3D12_HEAP_FLAG_ALLOW_ONLY_BUFFERS | D3D12_HEAP_FLAG_ALLOW_SHADER_ATOMICS,
            };
            let mut made: Option<ID3D12Heap> = None;
            // SAFETY: plain creation of a heap; the out pointer is ours.
            if let Err(e) = unsafe { host.device.CreateHeap(&raw const heap_desc, &raw mut made) } {
                tracing::warn!(error = %e, "no heap for host copies of title memory");
                return None;
            }
            let mut fresh = BufferHeap {
                heap: made?,
                ranges: RangePool::new(HEAP_BYTES),
                size: HEAP_BYTES,
            };
            let offset = fresh.ranges.take(len, PLAIN)?;
            FREE.fetch_add(HEAP_BYTES, Ordering::Relaxed);
            let index = match heaps.iter().position(Option::is_none) {
                Some(index) => {
                    heaps[index] = Some(fresh);
                    index
                }
                None => {
                    heaps.push(Some(fresh));
                    heaps.len() - 1
                }
            };
            tracing::debug!(index, size_mb = HEAP_BYTES >> 20, "a heap for host copies");
            (index, offset)
        }
    };
    FREE.fetch_sub(len, Ordering::Relaxed);
    let heap = heaps[index].as_ref()?.heap.clone();
    Some((
        heap,
        Cut {
            heap: index,
            offset,
            len,
        },
    ))
}

fn give_back(index: usize, offset: u64, len: u64) {
    let mut heaps = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(Some(heap)) = heaps.get_mut(index) else {
        return;
    };
    heap.ranges.give(offset, len);
    let free = FREE.fetch_add(len, Ordering::Relaxed) + len;
    if heap.ranges.room() == heap.size && free >= HEAP_KEPT + heap.size {
        FREE.fetch_sub(heap.size, Ordering::Relaxed);
        heaps[index] = None;
    }
}

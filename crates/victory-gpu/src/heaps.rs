//! Plain textures placed in shared heaps.
//!
//! After shadowdog00/vi-pc-combo: one committed resource per streamed texture
//! was most of the texture pass, and each allocation adds driver private
//! bytes on top of the texels. Textures the title fills and places, up to
//! 8 MiB, are cut from heaps of `VICTORY_TEXTURE_HEAP_MB` (64 by default).
//! A range comes back only once nothing else holds the resource and the
//! submissions that named it have finished, which is what keeps a later
//! texture from being bound over one the card is still reading.
//!
//! `VICTORY_TEXTURE_HEAPS=0` keeps a committed resource each. Unset, the
//! small-machine profile turns the heaps on.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Graphics::Direct3D12::*;

use crate::laptop::{RangePool, reserved_alignment};

/// The largest texture placed in a heap. Bigger ones stay committed, so one
/// of them cannot pin a whole heap.
const HEAPED_MOST: u64 = 8 << 20;
/// Free bytes kept when an empty heap is given back. One empty heap stays
/// so the next texture does not pay for creating it.
const HEAP_KEPT: u64 = 64 << 20;
/// 64 KiB, the only alignment these heaps cut on. 4 KiB is what reserved
/// textures on an 8 GB NVIDIA card were refused for.
const PLAIN: u64 = 64 << 10;

struct TextureHeap {
    heap: ID3D12Heap,
    ranges: RangePool,
    size: u64,
}

// SAFETY: a free-threaded D3D12 heap.
unsafe impl Send for TextureHeap {}

/// A range of a heap. Dropping it gives the bytes back.
pub(crate) struct Cut {
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

/// A placed texture whose owner is gone while something else still holds
/// the resource. The texture is released first, then the range.
struct Leaving {
    texture: ID3D12Resource,
    #[allow(dead_code)]
    cut: Cut,
}

// SAFETY: a free-threaded D3D12 resource and a range.
unsafe impl Send for Leaving {}

static HEAPS: Mutex<Vec<Option<TextureHeap>>> = Mutex::new(Vec::new());
static LEAVING: Mutex<Vec<Leaving>> = Mutex::new(Vec::new());
/// Bytes of the heaps that no texture is placed in.
static FREE: AtomicU64 = AtomicU64::new(0);

/// Whether new plain textures are placed in heaps.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = crate::laptop::texture_heaps_on(
            std::env::var("VICTORY_TEXTURE_HEAPS").ok().as_deref(),
            crate::tuning::profile().texture_heaps,
        );
        if on {
            tracing::info!(
                "plain textures share heaps (VICTORY_TEXTURE_HEAPS; =0: one committed resource each)"
            );
        }
        on
    })
}

fn heap_bytes() -> u64 {
    static SIZE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        crate::laptop::texture_heap_bytes(std::env::var("VICTORY_TEXTURE_HEAP_MB").ok().as_deref())
    })
}

/// Holds `texture` until it is the last reference and the card has finished
/// the submissions that were running, then gives its range back.
pub(crate) fn defer_release(texture: ID3D12Resource, cut: Cut) {
    LEAVING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Leaving { texture, cut });
}

/// Lets go of placed textures nothing else holds. Called from the idle sweep
/// and before a new range is cut.
pub(crate) fn reclaim() {
    let mut gone = Vec::new();
    {
        let mut leaving = LEAVING.lock().unwrap_or_else(|e| e.into_inner());
        let mut i = 0;
        while i < leaving.len() {
            if crate::host::references(&leaving[i].texture) == 1 {
                gone.push(leaving.swap_remove(i));
            } else {
                i += 1;
            }
        }
    }
    if !gone.is_empty() {
        crate::host::retire(gone);
    }
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

/// `desc` placed in a shared heap, in `state`, with the range that keeps
/// that memory for it. `None` when heaps are off, the texture is too big or
/// not 64 KiB aligned, or the card has no room for another heap: the caller
/// makes a committed texture.
pub(crate) fn place(
    host: &crate::host::Host,
    desc: &crate::resource::ResourceDesc,
    state: D3D12_RESOURCE_STATES,
) -> Option<(ID3D12Resource, Cut)> {
    if !enabled() {
        return None;
    }
    reclaim();
    // SAFETY: `desc` has the host resource-desc layout (the same cast
    // `host_texture_as` uses for committed textures).
    let mut gpu = unsafe {
        std::ptr::from_ref(desc)
            .cast::<D3D12_RESOURCE_DESC>()
            .read_unaligned()
    };
    gpu.Alignment = reserved_alignment(gpu.Alignment);
    // SAFETY: a plain size query.
    let info = unsafe { host.device.GetResourceAllocationInfo(0, &[gpu]) };
    if info.SizeInBytes == 0 || info.SizeInBytes > HEAPED_MOST || info.Alignment != PLAIN {
        return None;
    }
    gpu.Alignment = PLAIN;
    let len = info.SizeInBytes;
    let (heap, cut) = cut_range(host, len)?;
    let mut texture: Option<ID3D12Resource> = None;
    // SAFETY: `cut` is this texture's alone until it is dropped, and the
    // description is the one the size was asked for.
    let made = unsafe {
        host.device
            .CreatePlacedResource(&heap, cut.offset, &gpu, state, None, &mut texture)
    };
    match made.and_then(|()| texture.ok_or_else(windows::core::Error::empty)) {
        Ok(texture) => Some((texture, cut)),
        Err(e) => {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(
                || tracing::warn!(error = %e, "no placed host texture; a committed one instead"),
            );
            // `cut` drops here and the range goes back.
            None
        }
    }
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
            let size = heap_bytes();
            if let Some([budget, used, ..]) = host.video_memory() {
                let hundredths = crate::tuning::profile().room_percent.clamp(50, 97);
                if used + size > budget / 100 * hundredths {
                    return None;
                }
            }
            let heap_desc = D3D12_HEAP_DESC {
                SizeInBytes: size,
                Properties: D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_DEFAULT,
                    ..Default::default()
                },
                Alignment: reserved_alignment(0),
                Flags: D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
            };
            let mut made: Option<ID3D12Heap> = None;
            // SAFETY: plain creation of a heap; the out pointer is ours.
            if let Err(e) = unsafe { host.device.CreateHeap(&raw const heap_desc, &raw mut made) } {
                tracing::warn!(error = %e, "no heap for plain textures");
                return None;
            }
            let mut fresh = TextureHeap {
                heap: made?,
                ranges: RangePool::new(size),
                size,
            };
            let offset = fresh.ranges.take(len, PLAIN)?;
            FREE.fetch_add(size, Ordering::Relaxed);
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
            tracing::debug!(index, size_mb = size >> 20, "a heap for plain textures");
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

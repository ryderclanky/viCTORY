//! Policy for a 16 GB RAM / 8 GB VRAM laptop.
//!
//! Every switch here is off unless the environment asks, except shared texture
//! heaps, which follow the small-machine profile the way levels-on-demand do.
//! The decisions are pure so the host tests can lock them without a GPU.
//!
//! `VICTORY_TEXTURE_DROP_MIPS` follows soldsawn/victory-nvidia-8gb (and
//! fxceless/victory-fork #610): the host copy of a large 2D texture the title
//! fills leaves out its largest levels, and views and copies shift down so
//! level `m` on the host is still the title's level `m + N`. Linox-IDK/LNX-victory
//! drops the same texels by leaving the top of a reserved texture unmapped
//! (`VICTORY_TEXTURE_SKIP_MIPS`); this tree uses the shift, which keeps
//! subresource numbers inside the resource the host actually created.
//!
//! Reserved textures and the heaps they are cut from take the default 64 KiB
//! alignment. The console's 4 KiB alignment makes `CreateReservedResource`
//! fail, and retrying those as committed resources is what filled an 8 GB
//! card until the device was removed.

use victory_formats::texture::{Dimension, Layout};

/// How many levels `VICTORY_TEXTURE_DROP_MIPS` asks to leave out (0 to 4).
pub(crate) fn parse_drop_mips(raw: Option<&str>) -> u32 {
    raw.and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0)
        .min(4)
}

/// The title texture `drop_levels` looks at. Flags are the console resource
/// flags: render target, depth and unordered access are the low three bits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextureShape {
    pub(crate) dimension: u32,
    pub(crate) flags: u32,
    pub(crate) sample_count: u32,
    pub(crate) mip_levels: u32,
    pub(crate) width: u64,
    pub(crate) height: u64,
}

/// Levels the host copy of `shape` leaves out when `asked` were requested.
///
/// Only 2D textures the title fills (no target, depth or unordered-access
/// flag), including arrays and cubes, with at least five levels and 512
/// texels on the shorter side. Fewer than five levels keeps splash images
/// sharp. The sides must divide into whole 4-texel blocks after the shift,
/// so host level `m` is the title's level `m + n` texel for texel.
///
/// Textures at 256 or below (the ocean maps `VICTORY_TEXTURE_BACK` copies)
/// are left whole.
pub(crate) fn dropped_levels(asked: u32, shape: &TextureShape) -> u32 {
    const TEXTURE_2D: u32 = 3;
    const BUFFER: u32 = 1;
    /// Render target, depth, unordered access.
    const WRITTEN: u32 = 0x7;
    let asked = asked.min(4);
    if asked == 0
        || shape.dimension == BUFFER
        || shape.dimension != TEXTURE_2D
        || shape.flags & WRITTEN != 0
        || shape.sample_count > 1
        || shape.mip_levels < 5
    {
        return 0;
    }
    let n = asked.min(shape.mip_levels.saturating_sub(1));
    let whole = |side: u64| side.is_multiple_of(4u64 << n);
    if n == 0 || shape.width.min(shape.height) < 512 || !whole(shape.width) || !whole(shape.height)
    {
        return 0;
    }
    n
}

/// Width, height and level count of a host copy that leaves out `drop` levels.
pub(crate) fn shrunk(width: u64, height: u32, mips: u16, drop: u32) -> (u64, u32, u16) {
    if drop == 0 {
        return (width, height, mips);
    }
    (
        (width >> drop).max(1),
        (height >> drop).max(1),
        mips.saturating_sub(drop as u16).max(1),
    )
}

/// A shader view's first level, level count and smallest-level clamp on a
/// host texture that leaves out `drop` title levels. A count of `u32::MAX`
/// (every remaining level) stays.
pub(crate) fn shift_levels(drop: u32, first: &mut u32, levels: &mut u32, clamp: &mut f32) {
    if drop == 0 {
        return;
    }
    if *levels != u32::MAX {
        let end = first.saturating_add(*levels);
        *levels = end.saturating_sub(drop.max(*first)).max(1);
    }
    *first = first.saturating_sub(drop);
    *clamp = (*clamp - drop as f32).max(0.0);
}

/// Host subresource for title subresource `index` when `drop` levels are
/// left out. `None` for a level the host copy does not have.
pub(crate) fn host_subresource(drop: u32, mips: u32, index: u32) -> Option<u32> {
    let mips = mips.max(1);
    let (mip, slice) = (index % mips, index / mips);
    (mip >= drop).then(|| mip - drop + slice * (mips - drop))
}

/// Alignment `CreateReservedResource` and a heap of its tiles will take.
///
/// Always `0`, the default 64 KiB placement alignment. The console's 4 KiB
/// alignment is refused, so whatever the title asked is not passed on.
pub(crate) fn reserved_alignment(_asked: u64) -> u64 {
    0
}

/// `true` when `raw` is exactly `1`. Used for switches that must stay off
/// unless someone turns them on: ray tracing (`VICTORY_BVH_BUILDER`) and
/// in-flight card fetches (`VICTORY_CARD_FETCH_INFLIGHT`).
pub(crate) fn opt_in(raw: Option<&str>) -> bool {
    matches!(raw.map(str::trim), Some("1"))
}

/// `VICTORY_INPLACE`, off when unset. Sharing title memory in place made
/// frames slower (the card reads it over the bus); it stays off unless set.
pub(crate) fn inplace_level(raw: Option<&str>) -> u8 {
    raw.and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// `VICTORY_AMBIENT`: `None` leaves the title's indirect light alone (unset,
/// `off`, or not a finite level). A number is clamped to `0.0..=1.0`.
pub(crate) fn ambient_level(raw: Option<&str>) -> Option<f32> {
    let raw = raw?.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("off") {
        return None;
    }
    let level: f32 = raw.parse().ok()?;
    if !level.is_finite() || level < 0.0 {
        return None;
    }
    Some(level.min(1.0))
}

/// Whether plain textures share heaps (`VICTORY_TEXTURE_HEAPS`).
/// `1` / `on` forces them, `0` / `off` forces committed textures, and
/// anything else follows the machine profile.
pub(crate) fn texture_heaps_on(raw: Option<&str>, profile_on: bool) -> bool {
    match raw.map(str::trim) {
        Some("1" | "on") => true,
        Some("0" | "off") => false,
        _ => profile_on,
    }
}

/// `VICTORY_UI_PLANE`: `0` leaves the title's interface plane out,
/// `straight` blends it by straight alpha, and anything else (including
/// unset) uses premultiplied alpha, which is how the console composites
/// that plane over the picture. `0` off, `1` premultiplied, `2` straight.
pub(crate) fn ui_plane_mode(raw: Option<&str>) -> u8 {
    match raw.map(str::trim) {
        Some("0") => 0,
        Some("straight") => 2,
        _ => 1,
    }
}

/// Whether host copies of title memory share heaps (`VICTORY_BUFFER_HEAPS`).
/// `1` / `on` places them, anything else keeps one committed buffer each.
/// A tight machine does not turn this on: the shared heaps commit their
/// whole 64 MiB, and on the 16 GB laptop that raised private RAM.
pub(crate) fn buffer_heaps_on(raw: Option<&str>) -> bool {
    matches!(raw.map(str::trim), Some("1" | "on"))
}

/// A shared buffer of `size` bytes is cut from a heap when this is `Some`.
/// Under 64 KiB stays committed: a placed buffer is aligned to 64 KiB, so a
/// 256-byte one would reserve a whole page of the heap. Over 16 MiB stays
/// committed so one copy cannot pin a heap.
pub(crate) fn buffer_heap_len(size: u64) -> Option<u64> {
    const PLAIN: u64 = 64 << 10;
    const MOST: u64 = 16 << 20;
    ((PLAIN..=MOST).contains(&size) && size.is_multiple_of(PLAIN)).then_some(size)
}

/// The grid, in bytes, windows of asks that give a length are cut on.
/// Unset is 4 MiB. `VICTORY_WINDOW_KB` is a power of two from 64 KiB to
/// 4 MiB; anything else is the 4 MiB grid.
///
/// A finer grid was tried on the 16 GB laptop (1 MiB, with tails). Private
/// RAM rose from 17.3 GB to 18.5 GB and prison-cell frames went from 255 ms
/// to 571 ms, with 5–8 s stalls that removed the device. More windows were
/// made, copied, and let go. The stable layout is one 4 MiB window a cell.
pub(crate) fn window_grid_bytes(raw: Option<&str>) -> u64 {
    const WINDOW: u64 = 4 << 20;
    let Some(raw) = raw.map(str::trim) else {
        return WINDOW;
    };
    match raw.parse::<u64>().ok() {
        Some(kb) if kb.is_power_of_two() && (64..=4096).contains(&kb) => kb << 10,
        _ => WINDOW,
    }
}

/// Whether an ask with no length keeps only the tail of its 4 MiB cell
/// (`VICTORY_WINDOW_TAILS=1`). Off unless set. On a 4 MiB grid a sized ask
/// in the same cell still makes the whole cell, so a tail is a second
/// window of the same bytes. The laptop run that turned tails on with a
/// 1 MiB grid committed more and stalled.
pub(crate) fn window_tails_on(raw: Option<&str>) -> bool {
    matches!(raw.map(str::trim), Some("1" | "on"))
}

/// Microseconds each preparing part advanced from `then` to `now`.
///
/// A part that reads lower than the last present saturates at zero. The
/// frame-rate line used to zero those counters, and a wrapping subtraction
/// then printed a multi-hour part (34 of 103 gameplay frames on the laptop).
pub(crate) fn prep_delta(now: [u64; 6], then: [u64; 6]) -> [u64; 6] {
    std::array::from_fn(|i| now[i].saturating_sub(then[i]))
}

/// Adds `[from, to)` to `ranges`, which stays sorted and holds no two
/// ranges that overlap or touch. A gap of one byte or more is kept: those
/// bytes were not asked for, and copying them filled pages the copy had
/// never taken.
pub(crate) fn admit_range(ranges: &mut Vec<(usize, usize)>, from: usize, to: usize) {
    if from >= to {
        return;
    }
    let i = ranges.partition_point(|&(_, end)| end < from);
    let (mut from, mut to) = (from, to);
    while i < ranges.len() && ranges[i].0 <= to {
        from = from.min(ranges[i].0);
        to = to.max(ranges[i].1);
        ranges.remove(i);
    }
    ranges.insert(i, (from, to));
}

/// Groups of `pieces` `(key, source start, source end, destination)` that
/// are one copy: the same key, sources that abut, and a destination packed
/// against the piece before. The bytes are the same; one region is one
/// command instead of one per 64 KiB piece of a copy-back.
pub(crate) fn coalesce_groups<K: Eq>(
    pieces: &[(K, usize, usize, u64)],
) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    for i in 0..pieces.len() {
        let prev = i.checked_sub(1).map(|p| &pieces[p]);
        let join = prev.is_some_and(|prev| {
            prev.2 >= prev.1
                && pieces[i].0 == prev.0
                && pieces[i].1 == prev.2
                && pieces[i].3 == prev.3 + (prev.2 - prev.1) as u64
        });
        if !join {
            if i > start {
                groups.push(start..i);
            }
            start = i;
        }
    }
    if start < pieces.len() {
        groups.push(start..pieces.len());
    }
    groups
}

/// Bytes of snapshot buffers kept for reuse.
///
/// A tight machine keeps 128 MiB. Under memory pressure (under 2 GiB free)
/// that falls to 64 MiB, and under 4 GiB free a large machine also stops
/// at 128 MiB. Otherwise 512 MiB. One buffer of each size class is kept
/// even past the cap, so the large heap's snapshot is not thrown away and
/// allocated again (that allocation was the multi-second stall).
pub(crate) fn snapshot_pool_cap(tight: bool, free_ram: Option<u64>) -> usize {
    let base = if tight { 128 << 20 } else { 512 << 20 };
    match free_ram {
        Some(free) if free < 2 << 30 => 64 << 20,
        Some(free) if free < 4 << 30 => base.min(128 << 20),
        _ => base,
    }
}

/// The size class of a snapshot of `n` bytes: the next power of two, and
/// at least 4 KiB. A request and a buffer meet only in one class, so a
/// small copy is not handed the buffer kept for the big heap.
pub(crate) fn snapshot_class(n: usize) -> usize {
    n.max(4096)
        .checked_next_power_of_two()
        .unwrap_or(usize::MAX)
}

/// Spare snapshot buffers, one bucket per [`snapshot_class`].
#[derive(Clone, Debug, Default)]
pub(crate) struct SnapshotPool {
    pub(crate) buckets: std::collections::BTreeMap<usize, Vec<Vec<u8>>>,
    pub(crate) bytes: usize,
}

impl SnapshotPool {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.buckets.values().map(Vec::len).sum()
    }

    /// A spare buffer of at least `n` bytes from this request's class, or
    /// `None`. The caller allocates outside the lock when this is `None`.
    pub(crate) fn take(&mut self, n: usize) -> Option<Vec<u8>> {
        let class = snapshot_class(n);
        loop {
            let mut v = self.buckets.get_mut(&class)?.pop()?;
            self.bytes = self.bytes.saturating_sub(v.capacity());
            if self.buckets.get(&class).is_some_and(Vec::is_empty) {
                self.buckets.remove(&class);
            }
            if v.capacity() >= n {
                v.clear();
                return Some(v);
            }
        }
    }

    /// Keeps `v` when the pool is under `cap`, and always keeps the first
    /// buffer of its class. Extras past `cap` are dropped, largest class
    /// first, but never the last buffer of a class.
    pub(crate) fn give(&mut self, v: Vec<u8>, cap: usize) {
        if v.capacity() == 0 {
            return;
        }
        let class = snapshot_class(v.capacity());
        let bucket = self.buckets.entry(class).or_default();
        let first_of_class = bucket.is_empty();
        if !first_of_class && self.bytes.saturating_add(v.capacity()) > cap {
            return;
        }
        self.bytes = self.bytes.saturating_add(v.capacity());
        bucket.push(v);
        self.trim(cap);
    }

    /// Drops spare buffers until `bytes` is within `cap` or every class
    /// has a single buffer left.
    pub(crate) fn trim(&mut self, cap: usize) {
        while self.bytes > cap {
            let Some(class) = self
                .buckets
                .iter()
                .rev()
                .find(|(_, bucket)| bucket.len() > 1)
                .map(|(class, _)| *class)
            else {
                break;
            };
            let Some(v) = self.buckets.get_mut(&class).and_then(Vec::pop) else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(v.capacity());
            if self.buckets.get(&class).is_some_and(Vec::is_empty) {
                self.buckets.remove(&class);
            }
        }
    }
}

/// Bytes in one shared texture heap (`VICTORY_TEXTURE_HEAP_MB`, default 64,
/// clamped to 32..=256). 256 MiB is what a 16 GB card used; an 8 GB card
/// keeps a smaller heap so an empty tail is not a large part of its budget.
pub(crate) fn texture_heap_bytes(raw: Option<&str>) -> u64 {
    let mb = raw.and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(64);
    mb.clamp(32, 256) << 20
}

/// Free ranges inside one heap, lowest address first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RangePool {
    size: u64,
    free: Vec<(u64, u64)>,
}

impl RangePool {
    pub(crate) fn new(size: u64) -> Self {
        Self {
            size,
            free: if size == 0 {
                Vec::new()
            } else {
                vec![(0, size)]
            },
        }
    }

    pub(crate) fn room(&self) -> u64 {
        self.free.iter().map(|(_, n)| *n).sum()
    }

    /// `len` bytes at a multiple of `align` (a power of two). The gap in
    /// front of the aligned start stays free.
    pub(crate) fn take(&mut self, len: u64, align: u64) -> Option<u64> {
        if len == 0 || !align.is_power_of_two() {
            return None;
        }
        let mut i = 0;
        while i < self.free.len() {
            let (start, span) = self.free[i];
            let aligned = start.next_multiple_of(align);
            let pad = aligned.saturating_sub(start);
            if pad < span && span - pad >= len {
                let end = start + span;
                let used_end = aligned + len;
                self.free.remove(i);
                if aligned > start {
                    self.free.insert(i, (start, aligned - start));
                    i += 1;
                }
                if used_end < end {
                    self.free.insert(i, (used_end, end - used_end));
                }
                return Some(aligned);
            }
            i += 1;
        }
        None
    }

    /// Gives `[start, start + len)` back. Adjacent free ranges become one.
    /// A range that overlaps a free one, or that does not lie in the heap,
    /// is ignored.
    pub(crate) fn give(&mut self, start: u64, len: u64) {
        let Some(end) = start.checked_add(len) else {
            return;
        };
        if len == 0 || end > self.size {
            return;
        }
        if self.free.iter().any(|&(at, n)| start < at + n && at < end) {
            return;
        }
        let i = self.free.partition_point(|&(at, _)| at < start);
        if i > 0 {
            let (prev, prev_len) = self.free[i - 1];
            if prev + prev_len == start {
                self.free[i - 1].1 = prev_len + len;
                if i < self.free.len() {
                    let (next, next_len) = self.free[i];
                    if prev + prev_len + len == next {
                        self.free[i - 1].1 += next_len;
                        self.free.remove(i);
                    }
                }
                return;
            }
        }
        if i < self.free.len() {
            let (next, next_len) = self.free[i];
            if end == next {
                self.free[i] = (start, next_len + len);
                return;
            }
        }
        self.free.insert(i, (start, len));
    }
}

/// Staging layout for a host copy that leaves out `drop` levels.
///
/// A single 2D slice stores its smaller levels as a prefix. That prefix is
/// what the refresh reads, and the layout's levels are renumbered so host
/// level 0 is the title's first kept level. Arrays keep the original stride
/// and the caller samples from level `drop` (`first_mip`). If the dropped
/// levels are not a suffix of the allocation, nothing is cropped.
pub(crate) fn crop_staged_layout(mut layout: Layout, drop: u32) -> (Layout, u32) {
    if drop == 0 || layout.desc.dimension != Dimension::Tex2d || layout.slices != 1 {
        return (layout, drop);
    }
    if drop as usize >= layout.mips.len() {
        return (layout, drop);
    }
    let kept_end = layout.mips[drop as usize..]
        .iter()
        .map(|m| m.offset)
        .max()
        .unwrap_or(0);
    let dropped_start = layout.mips[..drop as usize]
        .iter()
        .map(|m| m.offset)
        .min()
        .unwrap_or(0);
    if dropped_start < kept_end {
        return (layout, drop);
    }
    let last = layout.mips[drop as usize - 1];
    let bytes = if last.tail.is_some() {
        layout.alignment
    } else {
        last.offset
    };
    if bytes == 0 || bytes > layout.size {
        return (layout, drop);
    }
    layout.mips.drain(..drop as usize);
    layout.first_tail_mip = layout.first_tail_mip.saturating_sub(drop);
    layout.desc.width = (layout.desc.width >> drop).max(1);
    layout.desc.height = (layout.desc.height >> drop).max(1);
    layout.desc.mips = layout.desc.mips.saturating_sub(drop).max(1);
    layout.size = bytes;
    layout.slice_size = bytes;
    (layout, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use victory_formats::texture::{Swizzle, Texture, TextureDesc};

    fn shape(width: u64, height: u64, mips: u32) -> TextureShape {
        TextureShape {
            dimension: 3,
            flags: 0,
            sample_count: 1,
            mip_levels: mips,
            width,
            height,
        }
    }

    #[test]
    fn drop_mips_defaults_off_and_clamps() {
        assert_eq!(parse_drop_mips(None), 0);
        assert_eq!(parse_drop_mips(Some("")), 0);
        assert_eq!(parse_drop_mips(Some("no")), 0);
        assert_eq!(parse_drop_mips(Some("2")), 2);
        assert_eq!(parse_drop_mips(Some("9")), 4);
    }

    #[test]
    fn large_streamed_textures_drop_their_top_levels() {
        assert_eq!(dropped_levels(0, &shape(4096, 4096, 12)), 0);
        assert_eq!(dropped_levels(1, &shape(4096, 4096, 12)), 1);
        assert_eq!(dropped_levels(2, &shape(2048, 1024, 10)), 2);
        let (w, h, mips) = shrunk(2048, 1024, 10, 2);
        assert_eq!((w, h, mips), (512, 256, 8));
    }

    #[test]
    fn water_splash_targets_and_odd_sizes_stay_whole() {
        // The ocean maps VICTORY_TEXTURE_BACK copies are 256 and below.
        assert_eq!(dropped_levels(2, &shape(256, 256, 8)), 0);
        // Splash images have a short chain.
        assert_eq!(dropped_levels(1, &shape(1920, 1080, 4)), 0);
        let mut target = shape(1024, 1024, 8);
        target.flags = 1;
        assert_eq!(dropped_levels(1, &target), 0);
        let mut depth = shape(1024, 1024, 8);
        depth.flags = 2;
        assert_eq!(dropped_levels(1, &depth), 0);
        let mut samples = shape(1024, 1024, 8);
        samples.sample_count = 4;
        assert_eq!(dropped_levels(1, &samples), 0);
        // 516 halved is 258, which is not a whole number of 4-texel blocks.
        assert_eq!(dropped_levels(1, &shape(516, 516, 8)), 0);
        let mut buffer = shape(4096, 1, 1);
        buffer.dimension = 1;
        assert_eq!(dropped_levels(2, &buffer), 0);
    }

    #[test]
    fn views_and_copies_address_the_levels_the_host_kept() {
        let shifted = |drop, first, levels, clamp| {
            let (mut f, mut l, mut c) = (first, levels, clamp);
            shift_levels(drop, &mut f, &mut l, &mut c);
            (f, l, c)
        };
        assert_eq!(shifted(1, 0, u32::MAX, 0.0), (0, u32::MAX, 0.0));
        assert_eq!(shifted(1, 0, 1, 0.0), (0, 1, 0.0));
        assert_eq!(shifted(1, 0, 10, 0.0), (0, 9, 0.0));
        assert_eq!(shifted(1, 3, 2, 2.5), (2, 2, 1.5));
        assert_eq!(shifted(2, 1, 1, 0.0), (0, 1, 0.0));
        assert_eq!(shifted(0, 3, 2, 1.0), (3, 2, 1.0));

        assert_eq!(host_subresource(2, 8, 0), None);
        assert_eq!(host_subresource(2, 8, 2), Some(0));
        assert_eq!(host_subresource(2, 8, 7), Some(5));
        // Slice 1, title level 2, in a chain of 8 that kept 6.
        assert_eq!(host_subresource(2, 8, 8 + 2), Some(6));
        assert_eq!(host_subresource(0, 8, 3), Some(3));
    }

    #[test]
    fn reserved_textures_never_keep_the_consoles_4k_alignment() {
        assert_eq!(reserved_alignment(0), 0);
        assert_eq!(reserved_alignment(4 << 10), 0);
        assert_eq!(reserved_alignment(64 << 10), 0);
        assert_eq!(reserved_alignment(3), 0);
        assert_ne!(reserved_alignment(4 << 10), 4 << 10);
    }

    #[test]
    fn dead_end_switches_and_ray_tracing_stay_off() {
        assert_eq!(inplace_level(None), 0);
        assert_eq!(inplace_level(Some("0")), 0);
        assert_eq!(inplace_level(Some("1")), 1);
        assert!(!opt_in(None));
        assert!(!opt_in(Some("0")));
        assert!(!opt_in(Some("on")));
        assert!(opt_in(Some("1")));
    }

    #[test]
    fn ambient_is_off_until_a_level_in_range_is_set() {
        assert_eq!(ambient_level(None), None);
        assert_eq!(ambient_level(Some("off")), None);
        assert_eq!(ambient_level(Some("nope")), None);
        assert_eq!(ambient_level(Some("-1")), None);
        assert_eq!(ambient_level(Some("0")), Some(0.0));
        assert_eq!(ambient_level(Some("0.25")), Some(0.25));
        assert_eq!(ambient_level(Some("0.6")), Some(0.6));
        assert_eq!(ambient_level(Some("4")), Some(1.0));
    }

    #[test]
    fn the_interface_plane_is_premultiplied_unless_told() {
        assert_eq!(ui_plane_mode(None), 1);
        assert_eq!(ui_plane_mode(Some("")), 1);
        assert_eq!(ui_plane_mode(Some("1")), 1);
        assert_eq!(ui_plane_mode(Some("0")), 0);
        assert_eq!(ui_plane_mode(Some("straight")), 2);
    }

    #[test]
    fn texture_heaps_follow_the_profile_unless_told() {
        assert!(!texture_heaps_on(None, false));
        assert!(texture_heaps_on(None, true));
        assert!(!texture_heaps_on(Some("0"), true));
        assert!(texture_heaps_on(Some("1"), false));
        assert!(texture_heaps_on(Some("on"), false));
        assert_eq!(texture_heap_bytes(None), 64 << 20);
        assert_eq!(texture_heap_bytes(Some("256")), 256 << 20);
        assert_eq!(texture_heap_bytes(Some("8")), 32 << 20);
        assert_eq!(texture_heap_bytes(Some("9999")), 256 << 20);
    }

    #[test]
    fn windows_stay_4_mib_unless_a_grid_is_named() {
        assert!(!buffer_heaps_on(None));
        assert!(!buffer_heaps_on(Some("0")));
        assert!(!buffer_heaps_on(Some("off")));
        assert!(buffer_heaps_on(Some("1")));
        assert!(buffer_heaps_on(Some("on")));

        assert_eq!(buffer_heap_len(256), None);
        assert_eq!(buffer_heap_len((64 << 10) - 1), None);
        assert_eq!(buffer_heap_len(64 << 10), Some(64 << 10));
        assert_eq!(buffer_heap_len(1 << 20), Some(1 << 20));
        assert_eq!(buffer_heap_len(16 << 20), Some(16 << 20));
        assert_eq!(buffer_heap_len((16 << 20) + (64 << 10)), None);
        assert_eq!(buffer_heap_len((1 << 20) + 256), None);

        assert_eq!(window_grid_bytes(None), 4 << 20);
        assert_eq!(window_grid_bytes(Some("4096")), 4 << 20);
        assert_eq!(window_grid_bytes(Some("1024")), 1 << 20);
        assert_eq!(window_grid_bytes(Some("256")), 256 << 10);
        assert_eq!(window_grid_bytes(Some("nope")), 4 << 20);

        assert!(!window_tails_on(None));
        assert!(!window_tails_on(Some("0")));
        assert!(!window_tails_on(Some("off")));
        assert!(window_tails_on(Some("1")));
        assert!(window_tails_on(Some("on")));

        assert_eq!(snapshot_pool_cap(false, None), 512 << 20);
        assert_eq!(snapshot_pool_cap(true, None), 128 << 20);
        assert_eq!(snapshot_pool_cap(true, Some(8 << 30)), 128 << 20);
        assert_eq!(snapshot_pool_cap(false, Some(3 << 30)), 128 << 20);
        assert_eq!(snapshot_pool_cap(true, Some(1 << 30)), 64 << 20);
        assert_eq!(snapshot_pool_cap(false, Some(1 << 30)), 64 << 20);
    }

    #[test]
    fn a_snapshot_pool_reuses_a_class_and_keeps_one_past_the_cap() {
        let mut pool = SnapshotPool::default();
        let cap = 128 << 20;
        let mut small = Vec::with_capacity(1 << 20);
        small.extend_from_slice(&[1, 2, 3]);
        pool.give(small, cap);
        let again = pool.take(1 << 20).expect("the 1 MiB class");
        assert!(again.capacity() >= 1 << 20);
        assert!(again.is_empty(), "a reused snapshot is cleared");
        pool.give(again, cap);

        let huge = Vec::with_capacity(174 << 20);
        pool.give(huge, cap);
        assert!(
            pool.take(64 << 10).is_none(),
            "a small copy is not handed the big heap's buffer"
        );
        let huge = pool.take(174 << 20).expect("the large class");
        assert!(huge.capacity() >= 174 << 20);
        pool.give(huge, cap);
        // Past the cap, extras of a class go; the last of each class stays.
        pool.give(Vec::with_capacity(1 << 20), cap);
        pool.give(Vec::with_capacity(1 << 20), cap);
        pool.trim(0);
        assert_eq!(pool.len(), 2, "one buffer of each class");
        assert!(pool.bytes() > cap);
        assert!(pool.take(1 << 20).is_some());
        assert!(pool.take(174 << 20).is_some());
    }

    #[test]
    fn a_preparing_part_that_goes_backwards_saturates_instead_of_wrapping() {
        // A frame-rate line that zeroed the counters left `then` large and
        // `now` small. Wrapping subtraction turned that into a multi-hour part.
        let then = [1_000, 50, 10_000_000, 0, 8, 3];
        let now = [1_225, 50, 4, 0, 7, 90];
        assert_eq!(prep_delta(now, then), [225, 0, 0, 0, 0, 87]);
        assert!(now[2].wrapping_sub(then[2]) > 1 << 32);
        assert!(now[4].wrapping_sub(then[4]) > 1 << 32);
    }

    #[test]
    fn a_refresh_keeps_the_gap_between_two_asks_and_joins_what_touches() {
        let mut ranges = Vec::new();
        admit_range(&mut ranges, 1_000, 1_100);
        admit_range(&mut ranges, 4_000, 4_200);
        // The pages between the asks are not part of either refresh.
        assert_eq!(ranges, [(1_000, 1_100), (4_000, 4_200)]);
        let copied: usize = ranges.iter().map(|(from, to)| to - from).sum();
        assert_eq!(copied, 300);
        // Touching and overlapping asks become one. A later ask that
        // bridges the gap does too.
        admit_range(&mut ranges, 1_100, 1_150);
        admit_range(&mut ranges, 3_900, 4_050);
        assert_eq!(ranges, [(1_000, 1_150), (3_900, 4_200)]);
        admit_range(&mut ranges, 1_150, 3_900);
        assert_eq!(ranges, [(1_000, 4_200)]);
        admit_range(&mut ranges, 50, 50);
        assert_eq!(ranges, [(1_000, 4_200)]);
    }

    #[test]
    fn abutting_fetch_pieces_of_one_buffer_are_one_copy() {
        // (buffer, source start, source end, destination)
        let pieces = [
            (1usize, 0, 64 << 10, 0u64),
            (1, 64 << 10, 128 << 10, (64 << 10) as u64),
            (1, 192 << 10, 256 << 10, (128 << 10) as u64),
            (2, 0, 4096, (192 << 10) as u64),
            (2, 4096, 8192, ((192 << 10) + 4096) as u64),
        ];
        let groups = coalesce_groups(&pieces);
        assert_eq!(groups, [0..2, 2..3, 3..5]);
        // The joined copy covers the same source bytes at the same destination.
        let joined: Vec<(usize, usize, usize, u64)> = groups
            .iter()
            .map(|group| {
                let (key, lo, _, at) = pieces[group.start];
                let hi = pieces[group.end - 1].2;
                (key, lo, hi, at)
            })
            .collect();
        assert_eq!(
            joined,
            [
                (1, 0, 128 << 10, 0),
                (1, 192 << 10, 256 << 10, (128 << 10) as u64),
                (2, 0, 8192, (192 << 10) as u64),
            ]
        );
        // A destination that is not packed stays split.
        let split = [(7usize, 0, 10, 0u64), (7, 10, 20, 0u64)];
        assert_eq!(coalesce_groups(&split), [0..1, 1..2]);
        assert!(coalesce_groups::<(usize, usize, usize, u64)>(&[]).is_empty());
    }

    #[test]
    fn a_heap_reuses_a_range_only_after_it_is_given_back() {
        let mut pool = RangePool::new(256);
        let first = pool.take(100, 64).unwrap();
        assert_eq!(first, 0);
        let second = pool.take(64, 64).unwrap();
        assert_eq!(second, 128);
        assert!(pool.take(128, 64).is_none());
        assert_eq!(pool.room(), 256 - 100 - 64);
        pool.give(first, 100);
        // The returned range joins the gap in front of the second cut.
        assert!(pool.room() > 100);
        let again = pool.take(64, 64).unwrap();
        assert_eq!(again, 0);
        // A second give of the same bytes is ignored.
        let before = pool.room();
        pool.give(again, 64);
        pool.give(again, 64);
        assert_eq!(pool.room(), before + 64);
    }

    #[test]
    fn cropping_a_chain_keeps_the_smaller_levels_prefix() {
        let tex = Texture::new(&TextureDesc {
            dimension: Dimension::Tex2d,
            width: 512,
            height: 512,
            depth_or_array: 1,
            mips: 8,
            format: 98,
            samples: 1,
            swizzle: Swizzle::SW_64KB_S,
        })
        .expect("a console layout");
        let layout = tex.planes[0].layout.clone();
        let full = layout.size;
        let (cropped, first) = crop_staged_layout(layout.clone(), 1);
        assert_eq!(first, 0);
        assert!(cropped.size < full);
        assert_eq!(cropped.mips.len(), layout.mips.len() - 1);
        assert_eq!(cropped.mips[0].offset, layout.mips[1].offset);
        assert_eq!(cropped.mips[0].width, layout.mips[1].width);
        // Arrays keep their stride and say which level to start at.
        let mut array = layout;
        array.slices = 6;
        array.desc.depth = 6;
        let (same, first) = crop_staged_layout(array.clone(), 2);
        assert_eq!(first, 2);
        assert_eq!(same.size, array.size);
        assert_eq!(crop_staged_layout(array, 0).1, 0);
    }
}

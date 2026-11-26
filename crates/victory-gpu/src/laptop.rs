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

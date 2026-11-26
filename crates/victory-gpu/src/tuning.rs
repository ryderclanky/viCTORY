//! The memory this machine has, and the defaults it sets.
//!
//! This fork was measured on a PC with 32 GB of memory and a 12 GB card, where the process
//! holds about 24 GB (16 GB of it resident) and the card runs at 9.0 of a 10.2 GB budget. On a
//! smaller machine the same working set does not fit: Windows pages (one such run wrote 17.9 GB
//! to the page file), the driver's shader compiler fails with "ptxas fatal: Memory allocation
//! failure" and pipeline creation returns `0x8007000E`, and the title then traps in its own
//! fatal handler. Nothing in the code was wrong for the machine it was tuned on; the defaults
//! were.
//!
//! So the defaults are sized from what is actually there: the memory the system reports and the
//! card's DXGI budget, both asked once. Every value is still overridden by its switch, and a
//! machine with no card budget or no reading at all (a hosted test runner) gets exactly the
//! values this fork shipped with, so nothing changes where nothing is known.
//!
//! `VICTORY_PROFILE=0` keeps the shipped defaults everywhere; `VICTORY_PROFILE=small|large`
//! forces one of the two profiles (for testing on a machine that is not the one it describes).

use std::sync::OnceLock;

/// What the machine has, and the defaults that follow from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Profile {
    /// Physical memory, in GiB (0: not read).
    pub(crate) ram_gb: u64,
    /// The card's own budget from DXGI, in MiB (0: not read, or no card).
    pub(crate) budget_mb: u64,
    /// Room kept on the card for render targets and host copies the host itself writes, in MiB
    /// (`VICTORY_CARD_ROOM_FOR_TARGETS_MB`).
    pub(crate) targets_room_mb: u64,
    /// The share of the card's budget, in percent, up to which host copies of textures are
    /// made (`VICTORY_TEXTURE_ROOM_PERCENT`).
    pub(crate) room_percent: u64,
    /// Whether host copies of title textures take the card's memory level by level
    /// (`VICTORY_LEVELS_ON_DEMAND`), which holds back the levels the title does not hold: 2.2
    /// to 3.0 GB on the reference machine's runs.
    pub(crate) levels_on_demand: bool,
    /// The most staging buffers kept spare, in MiB (`VICTORY_STAGING_POOL_MB`).
    pub(crate) staging_mb: u64,
}

/// The defaults this fork shipped with, for a machine nothing is known about: the reference
/// machine's values.
const LARGE: Profile = Profile {
    ram_gb: 0,
    budget_mb: 0,
    targets_room_mb: 3_072,
    room_percent: 90,
    levels_on_demand: false,
    staging_mb: 256,
};

/// The profile a machine with `ram_gb` of memory and a card budget of `budget_mb` gets.
///
/// - **Room for render targets** is the largest single reserve on a small card: 3 GB of an
///   8 GB card's 6.5 GB budget is most of it, and the texture queue is refused from the first
///   world frame. A quarter of the budget keeps the same *proportion* the reference machine
///   had (3,072 of 10,200 is 30 %) without taking the card.
/// - **The texture room's share** goes up a little on a small card, to 92 %: at 90 % about a
///   tenth of the budget stays unused while thousands of textures wait, and eviction and
///   recreation cost far more here than the headroom they keep.
/// - **Levels on demand** is the largest memory win there is (2.2 to 3.0 GB of copies that hold
///   levels the title does not have) and it is the difference between a card that thrashes and
///   one that does not, below about an 8 GB budget. It runs on tiled-resource tier 2 only, which
///   every card of that size has.
/// - **Staging spares** are halved when the system itself is short of memory, because the pool
///   is PC memory (a quarter of a GiB is 1.6 % of 16 GB) held against a page file that is
///   already in use.
pub(crate) fn choose(ram_gb: u64, budget_mb: u64) -> Profile {
    if budget_mb == 0 {
        // Nothing read: the values this fork shipped with.
        return LARGE;
    }
    let small = budget_mb < 8_192 || (ram_gb != 0 && ram_gb <= 20);
    if !small {
        return Profile {
            ram_gb,
            budget_mb,
            ..LARGE
        };
    }
    Profile {
        ram_gb,
        budget_mb,
        targets_room_mb: (budget_mb / 4).clamp(512, 3_072),
        room_percent: 92,
        levels_on_demand: true,
        staging_mb: if ram_gb != 0 && ram_gb <= 20 { 64 } else { 256 },
    }
}

/// Physical memory in GiB, 0 if it cannot be read.
#[cfg(windows)]
fn ram_gb() -> u64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: a valid out pointer of the size its length field says.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return 0;
    }
    m.ullTotalPhys >> 30
}

#[cfg(not(windows))]
fn ram_gb() -> u64 {
    0
}

/// The commit the system still has to give, in bytes: what is left of the commit limit after what
/// every process has committed, page file included (`ullAvailPageFile`). `None` where it cannot be
/// read, which is "nothing known" as everywhere else here.
///
/// This is what a compile has to fit into, not the free physical memory: the run that died in the
/// driver's compiler had 645 MiB of this and 2.06 GB of a 47.7 GB page file free, and `ptxas`
/// could not allocate.
#[cfg(windows)]
pub(crate) fn commit_free() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: a valid out pointer of the size its length field says.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return None;
    }
    Some(m.ullAvailPageFile)
}

#[cfg(not(windows))]
pub(crate) fn commit_free() -> Option<u64> {
    None
}

/// The card's own budget in MiB, 0 if there is no card to ask (yet).
#[cfg(windows)]
fn budget_mb() -> u64 {
    crate::host::host()
        .and_then(|h| h.video_memory())
        .map_or(0, |v| v[0] >> 20)
}

#[cfg(not(windows))]
fn budget_mb() -> u64 {
    0
}

/// The machine's profile, read once: `VICTORY_PROFILE=0` keeps the shipped defaults,
/// `=small` and `=large` force one, anything else (or nothing) reads the machine.
pub(crate) fn profile() -> &'static Profile {
    static PROFILE: OnceLock<Profile> = OnceLock::new();
    PROFILE.get_or_init(|| {
        let forced = std::env::var("VICTORY_PROFILE").ok();
        let (read, forced) = match forced.as_deref().map(str::trim) {
            Some("0") | Some("large") => (LARGE, true),
            Some("small") => (choose(16, 6_500), true),
            _ => {
                let (ram, budget) = (ram_gb(), budget_mb());
                (choose(ram, budget), false)
            }
        };
        tracing::info!(
            ram_gb = read.ram_gb,
            budget_mb = read.budget_mb,
            forced,
            shipped = read == LARGE,
            targets_room_mb = read.targets_room_mb,
            room_percent = read.room_percent,
            levels_on_demand = read.levels_on_demand,
            staging_mb = read.staging_mb,
            "the memory this machine has, and the defaults it sets (VICTORY_PROFILE=0: the shipped ones)"
        );
        read
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_nothing_is_known_about_gets_the_shipped_defaults() {
        assert_eq!(choose(0, 0), LARGE);
        // a card budget read but no memory reading: the same knobs, sized from the card
        assert_eq!(choose(0, 0), LARGE);
    }

    #[test]
    fn the_reference_machine_keeps_the_measured_values() {
        // 32 GB and a 10.2 GB budget: what this fork's numbers were measured with.
        let p = choose(32, 10_200);
        assert_eq!(p.targets_room_mb, 3_072);
        assert_eq!(p.room_percent, 90);
        assert!(
            !p.levels_on_demand,
            "levels on demand stays off, as measured"
        );
        assert_eq!(p.staging_mb, 256);
    }

    #[test]
    fn a_small_card_keeps_a_quarter_of_its_budget_for_targets() {
        // 8 GB card, 6.5 GB budget, 16 GB of memory: this session's machine.
        let p = choose(16, 6_500);
        assert_eq!(p.targets_room_mb, 1_625);
        assert_eq!(p.room_percent, 92);
        assert!(p.levels_on_demand);
        assert_eq!(p.staging_mb, 64);
        // a 4 GB card keeps its floor, never nothing
        assert_eq!(choose(16, 3_800).targets_room_mb, 950);
        assert_eq!(choose(16, 1_000).targets_room_mb, 512);
    }

    #[test]
    fn plenty_of_memory_with_a_small_card_still_sizes_the_card() {
        // 64 GB of memory does not make an 8 GB card bigger.
        let p = choose(64, 6_500);
        assert!(p.levels_on_demand);
        assert_eq!(p.staging_mb, 256, "memory is not short: the pool stays");
    }
}

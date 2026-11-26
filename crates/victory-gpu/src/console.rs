//! Which console the title runs on. Titles pick settings (resolution,
//! memory budgets) by it, so the user chooses; Series X unless told.

use std::sync::atomic::{AtomicBool, Ordering};

const GIB: usize = 1 << 30;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Console {
    #[default]
    SeriesX,
    SeriesS,
}

impl Console {
    /// Title memory in bytes: (GPU-optimal, standard). Series X in the
    /// advanced configuration; Series S has one range, counted as standard
    /// (7,776 MiB, the published console-mode figure).
    // shortcut: later system updates gave Series S titles more (amount not
    // published here); the title sizes nothing from this figure
    // (it goes by the device type), so update it if a title does.
    pub const fn title_memory(self) -> (usize, usize) {
        match self {
            Console::SeriesX => (10 * GIB, 7 * GIB / 2),
            Console::SeriesS => (0, 7_776 << 20),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Console::SeriesX => "Xbox Series X",
            Console::SeriesS => "Xbox Series S",
        }
    }
}

static SERIES_S: AtomicBool = AtomicBool::new(false);

/// Set before the title starts.
pub fn set(console: Console) {
    SERIES_S.store(console == Console::SeriesS, Ordering::Relaxed);
}

pub fn current() -> Console {
    if SERIES_S.load(Ordering::Relaxed) {
        Console::SeriesS
    } else {
        Console::SeriesX
    }
}

/// `VICTORY_CONSOLE`: `series-s` (or `s`) is the smaller console, `series-x`
/// (or `x`) the larger one. Anything else is unset.
pub fn named(name: Option<&str>) -> Option<Console> {
    match name
        .map(str::trim)
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("series-s" | "series_s" | "s") => Some(Console::SeriesS),
        Some("series-x" | "series_x" | "x") => Some(Console::SeriesX),
        _ => None,
    }
}

/// The console the title sees. An explicit flag wins; otherwise
/// [`named`] of `VICTORY_CONSOLE`; otherwise Series X.
pub fn resolve(flag_series_s: Option<bool>, env: Option<&str>) -> Console {
    match flag_series_s {
        Some(true) => Console::SeriesS,
        Some(false) => Console::SeriesX,
        None => named(env).unwrap_or(Console::SeriesX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_s_is_the_16_gb_consoles_memory() {
        let (gpu, standard) = Console::SeriesS.title_memory();
        assert_eq!(gpu, 0);
        assert_eq!(standard, 7_776 << 20);
        assert!(standard < 8 << 30, "Series S title memory fits under 8 GiB");
        let (gpu, standard) = Console::SeriesX.title_memory();
        assert!(
            gpu + standard > 12 << 30,
            "Series X title memory is what overflowed a 16 GB machine"
        );
    }

    #[test]
    fn series_s_leaves_a_16_gb_laptop_room_the_layer_can_use() {
        let title = |c: Console| {
            let (gpu, standard) = c.title_memory();
            gpu + standard
        };
        let series_s = title(Console::SeriesS);
        let series_x = title(Console::SeriesX);
        // 16 GB of RAM. Series S's own limit is under 8 GiB, so the layer's
        // copies have the rest of the machine. Series X's limit is most of it
        // (the 21.5 GB private peak on that laptop).
        assert!((16 << 30) - series_s > 8 << 30);
        assert!(series_x > series_s + (4 << 30));
        assert!(series_s < 14 << 30);
    }

    #[test]
    fn the_flag_wins_and_the_env_selects_series_s() {
        assert_eq!(resolve(None, None), Console::SeriesX);
        assert_eq!(resolve(None, Some("series-s")), Console::SeriesS);
        assert_eq!(resolve(None, Some("S")), Console::SeriesS);
        assert_eq!(resolve(Some(false), Some("series-s")), Console::SeriesX);
        assert_eq!(resolve(Some(true), None), Console::SeriesS);
        assert_eq!(named(Some("nope")), None);
    }
}

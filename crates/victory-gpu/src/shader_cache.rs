//! Translated shaders kept on disk (decision 0003): under the configured
//! cache folder, keyed by the container's content and by the translator
//! that translated it (its source fingerprint and the settings that change
//! the DXIL, see `build`), so a changed translator never serves an old
//! translation. Entries go through `victory-pipeline`'s content-addressed
//! cache, which writes them atomically; an entry that does not read back is
//! translated again.
//!
//! The key does not include the whole environment. A diagnostics path
//! (`VICTORY_SHADER_CENSUS`, `VICTORY_DXIL`), a debug flag, or a laptop
//! memory switch (`VICTORY_PROFILE`, `VICTORY_TEXTURE_DROP_MIPS`,
//! `VICTORY_AMBIENT`) does not change the DXIL, and putting the raw value
//! in the key made every boot miss when that value changed. Host facts that
//! do change the DXIL (wave size, view-heap null slot) are saved beside the
//! entries so the next boot, and `victory translate --warm`, file them
//! under the same key the game will look up.
//!
//! The entry is our own encoding of [`HostShader`]: a field added to it
//! must be added to `encode` and `decode` too (entries of older builds
//! are ignored anyway).

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use victory_pipeline::{Cache, Converter};
use victory_shader::emit::{HostShader, StaticSampler, UserData};
use victory_shader::resource::RootIndexedView;
use victory_shader::{HwStage, RootType};

static CACHE: OnceLock<Cache> = OnceLock::new();
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

/// Keeps translated shaders in `dir` from now on. A facts file left by an
/// earlier run is applied first, so this process looks up shaders under the
/// same wave size and view-heap limit that wrote them.
pub fn use_shader_cache(dir: PathBuf) {
    match Cache::open(dir) {
        Ok(cache) => {
            apply_saved_facts(cache.root());
            let _ = CACHE.set(cache);
        }
        Err(e) => tracing::warn!(error = %e, "no shader cache"),
    }
}

/// Shaders found in the cache and translated since the start.
pub(crate) fn counts() -> (u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed))
}

/// What identifies the translation, besides the container: the shader
/// crate's fingerprint (its source, the switches that change the DXIL, the
/// host facts the device gave it) and this file's encoding of the result.
/// Not the whole executable: a rebuild that leaves the translator alone keeps
/// its translations (each new build used to translate the title's thousands
/// of shaders again). `VICTORY_SHADER_CACHE_KEY=exe` restores the old key
/// (the executable's hash). Worked out per lookup, so a translation made
/// before the device said its wave size is never filed under the later key.
fn build() -> [u8; 32] {
    static WARNED: std::sync::Once = std::sync::Once::new();
    let limit = victory_shader::emit::loop_limit();
    if limit != victory_shader::emit::DEFAULT_LOOP_LIMIT {
        WARNED.call_once(|| {
            tracing::warn!(
                limit,
                "VICTORY_SHADER_LOOP_LIMIT: translated loops stop sooner"
            )
        });
    }
    if let Some(cache) = CACHE.get() {
        remember_facts(cache.root());
    }
    static EXE: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    let exe = EXE.get_or_init(|| {
        if std::env::var_os("VICTORY_SHADER_CACHE_KEY").is_some_and(|v| v == "exe") {
            match std::env::current_exe().and_then(std::fs::read) {
                Ok(bytes) => Some(*blake3::hash(&bytes).as_bytes()),
                Err(e) => {
                    tracing::warn!(error = %e, "cannot read our executable; keying on the translator");
                    None
                }
            }
        } else {
            None
        }
    });
    let mut h = blake3::Hasher::new();
    h.update(b"victory shader cache key 3\0");
    if let Some(exe) = exe {
        h.update(exe);
    }
    h.update(&victory_shader::emit::translation_fingerprint());
    // The list this file reads for the translator (`translate_listed`): the
    // shader crate's fingerprint does not see it. UTF-8, not the platform's
    // opaque `OsStr` bytes: those are not a stable encoding to store.
    h.update(b"VICTORY_WAVE_WIDE_ONLY=");
    if let Some(value) = std::env::var_os("VICTORY_WAVE_WIDE_ONLY") {
        h.update(value.to_string_lossy().as_bytes());
    }
    h.update(b"encoding-v3");
    *h.finalize().as_bytes()
}

const FACTS_FILE: &str = "host-facts-v1";
const FACTS_MAGIC: &[u8; 4] = b"VHF1";

/// Wave lanes and the view-heap null slot a previous run translated with.
fn read_facts(dir: &std::path::Path) -> Option<(u32, u32)> {
    let bytes = std::fs::read(dir.join(FACTS_FILE)).ok()?;
    if bytes.len() != 12 || &bytes[..4] != FACTS_MAGIC {
        return None;
    }
    let wave = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let view = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    Some((wave, view))
}

fn write_facts(dir: &std::path::Path, wave: u32, view: u32) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(12);
    bytes.extend_from_slice(FACTS_MAGIC);
    bytes.extend_from_slice(&wave.to_le_bytes());
    bytes.extend_from_slice(&view.to_le_bytes());
    let path = dir.join(FACTS_FILE);
    let tmp = dir.join(".host-facts.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)
}

/// Fills in host facts the device has not set yet, from the last run.
fn apply_saved_facts(dir: &std::path::Path) {
    let Some((wave, view)) = read_facts(dir) else {
        return;
    };
    if victory_shader::emit::host_wave_lanes() == 0 && wave != 0 {
        victory_shader::emit::set_host_max_wave(wave);
    }
    if victory_shader::emit::view_heap_limit() == 0 && view != 0 {
        victory_shader::emit::set_view_heap_limit(view);
    }
}

/// Records the facts this run is translating with, once they are known, so
/// the next boot and an offline warm-up use the same key.
fn remember_facts(dir: &std::path::Path) {
    static SAVED: AtomicU64 = AtomicU64::new(u64::MAX);
    let wave = victory_shader::emit::host_wave_lanes();
    let view = victory_shader::emit::view_heap_limit();
    if wave == 0 && view == 0 {
        return;
    }
    let packed = (u64::from(wave) << 32) | u64::from(view);
    if SAVED.swap(packed, Ordering::Relaxed) == packed {
        return;
    }
    if let Err(e) = write_facts(dir, wave, view) {
        tracing::warn!(error = %e, "shader cache host facts not saved");
    }
}

/// The container bytes the translator reads. A DXBC blob's own size is at
/// byte 0x18; bytes past it are not part of the container (alignment slack,
/// or whatever shared the allocation). Keying on that slack made every boot
/// miss when the tail was not the same bytes.
fn shader_bytes(container: &[u8]) -> &[u8] {
    if container.len() >= 0x1C && container.starts_with(b"DXBC") {
        let total =
            u32::from_le_bytes(container[0x18..0x1C].try_into().expect("four size bytes")) as usize;
        if (0x20..=container.len()).contains(&total) {
            return &container[..total];
        }
    }
    container
}

/// A container translated with the wave-wide form allowed when its hash is
/// on `VICTORY_WAVE_WIDE_ONLY` (16-hex-digit hashes as the log names
/// shaders, comma separated; unset or empty: none). Off by default: two
/// programs that moved to that form were in a submission that lost the card
/// (runs 121-124, not proven the cause). After commit a416e392 of pull
/// request #823 (Felix2180), whose list is the four refused programs
/// 5dc3b25986622949 and 6f225b641c5b0708 (the water's transmission
/// volumetrics), 369cbe906ea72764 (particle visibility) and
/// 2cc4c197ee29e291 (the reflection pass's tiling). For runs and the offline
/// warm-up alike, which share the cache.
fn translate_listed(c: &[u8]) -> Result<HostShader, String> {
    static LIST: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| {
        std::env::var("VICTORY_WAVE_WIDE_ONLY")
            .unwrap_or_default()
            .split(',')
            .map(|h| h.trim().to_lowercase())
            .filter(|h| !h.is_empty() && h != "none")
            .collect()
    });
    let hash = blake3::hash(c).to_hex();
    let wide = list.iter().any(|h| hash.starts_with(h.as_str()));
    victory_shader::emit::with_wave_wide(wide, || victory_shader::emit::translate(c))
        .map_err(|e| e.to_string())
}

/// The offline warm-up (`victory translate --warm`): `container` is
/// translated into the cache unless it is there already. `Ok(true)`: it
/// was translated now. Needs [`use_shader_cache`] and the same host facts
/// the run will have (`victory_shader::emit::translation_fingerprint`).
pub fn warm(container: &[u8]) -> Result<bool, String> {
    let cache = CACHE.get().ok_or("no shader cache")?;
    let ran = AtomicBool::new(false);
    let translate = |c: &[u8]| {
        ran.store(true, Ordering::Relaxed);
        translate_listed(c)
    };
    cached(cache, &build(), container, translate).map(|_| ran.load(Ordering::Relaxed))
}

/// The translation of a console shader container, from the cache when
/// there is one.
pub(crate) fn host_shader(container: &[u8]) -> Result<HostShader, String> {
    let translate = translate_listed;
    match CACHE.get() {
        Some(cache) => cached(cache, &build(), container, translate),
        None => translate(container),
    }
}

/// The cache entry's converter: the build and the container in, the
/// encoded translation out. `ran` tells a miss from a hit.
struct Translate<F> {
    translate: F,
    ran: AtomicBool,
}

impl<F: Fn(&[u8]) -> Result<HostShader, String> + Send + Sync> Converter for Translate<F> {
    fn id(&self) -> &'static str {
        "shader.dxil"
    }
    fn version(&self) -> u32 {
        // The build fingerprint in the input does the versioning.
        1
    }
    fn convert(&self, input: &[u8]) -> anyhow::Result<Vec<u8>> {
        self.ran.store(true, Ordering::Relaxed);
        let shader = (self.translate)(&input[32..]).map_err(anyhow::Error::msg)?;
        Ok(encode(&shader))
    }
}

fn cached(
    cache: &Cache,
    build: &[u8; 32],
    container: &[u8],
    translate: impl Fn(&[u8]) -> Result<HostShader, String> + Send + Sync,
) -> Result<HostShader, String> {
    let container = shader_bytes(container);
    let input = [&build[..], container].concat();
    let converter = Translate {
        translate,
        ran: AtomicBool::new(false),
    };
    for _ in 0..2 {
        let path = cache
            .get_or_convert(&converter, &input)
            .map_err(|e| format!("{e:#}"))?;
        if let Some(shader) = std::fs::read(&path).ok().and_then(|b| decode(&b)) {
            let (counter, what) = if converter.ran.load(Ordering::Relaxed) {
                (&MISSES, "translated and cached")
            } else {
                (&HITS, "from the cache")
            };
            counter.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(path = %path.display(), "shader {what}");
            return Ok(shader);
        }
        tracing::debug!(path = %path.display(), "unreadable shader cache entry; translating again");
        let _ = std::fs::remove_file(&path);
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    (converter.translate)(container)
}

const MAGIC: &[u8; 4] = b"VSH1";
const STAGES: [HwStage; 8] = [
    HwStage::Ls,
    HwStage::Hs,
    HwStage::Es,
    HwStage::Gs,
    HwStage::Vs,
    HwStage::Ps,
    HwStage::Ms,
    HwStage::Cs,
];

fn root_code(r: RootType) -> u32 {
    match r {
        RootType::Constants => 0,
        RootType::Descriptor => 1,
        RootType::Table => 2,
        RootType::ImmediateSampler => 3,
        RootType::ResourceHeap => 4,
        RootType::SamplerHeap => 5,
        RootType::Other(x) => 6 + x,
    }
}

fn root_type(code: u32) -> RootType {
    match code {
        0 => RootType::Constants,
        1 => RootType::Descriptor,
        2 => RootType::Table,
        3 => RootType::ImmediateSampler,
        4 => RootType::ResourceHeap,
        5 => RootType::SamplerHeap,
        x => RootType::Other(x - 6),
    }
}

/// Little-endian words and length-prefixed lists.
fn encode(s: &HostShader) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    let word = |out: &mut Vec<u8>, v: u32| out.extend(v.to_le_bytes());
    let stage = STAGES.iter().position(|&x| x == s.stage).unwrap_or(0) as u32;
    word(&mut out, stage);
    word(&mut out, s.dxil.len() as u32);
    out.extend(&s.dxil);
    word(&mut out, s.user_data.len() as u32);
    for u in &s.user_data {
        for v in [u.index, u32::from(u.slot), root_code(u.root), u.dword] {
            word(&mut out, v);
        }
    }
    word(&mut out, s.samplers.len() as u32);
    for sampler in &s.samplers {
        for &v in sampler {
            word(&mut out, v);
        }
    }
    word(&mut out, s.static_samplers.len() as u32);
    for t in &s.static_samplers {
        let [u, v, w] = t.address;
        let words = [
            t.filter,
            u,
            v,
            w,
            t.mip_lod_bias.to_bits(),
            t.max_anisotropy,
            t.comparison,
            t.border,
            t.min_lod.to_bits(),
            t.max_lod.to_bits(),
        ];
        for v in words {
            word(&mut out, v);
        }
    }
    for slots in [&s.root_cbvs, &s.root_srvs, &s.root_uavs] {
        word(&mut out, slots.len() as u32);
        out.extend(slots);
    }
    word(&mut out, s.root_indexed_views.len() as u32);
    for view in &s.root_indexed_views {
        for value in [
            u32::from(view.table_slot),
            u32::from(view.constant_slot),
            view.constant_dword,
            view.byte_offset,
            u32::from(view.shift),
        ] {
            word(&mut out, value);
        }
    }
    word(&mut out, s.half_views.len() as u32);
    for &(slot, dword) in &s.half_views {
        word(&mut out, u32::from(slot));
        word(&mut out, dword);
    }
    out
}

fn decode(bytes: &[u8]) -> Option<HostShader> {
    struct Reader<'a>(&'a [u8]);
    impl<'a> Reader<'a> {
        fn take(&mut self, n: usize) -> Option<&'a [u8]> {
            let (head, rest) = self.0.split_at_checked(n)?;
            self.0 = rest;
            Some(head)
        }
        fn word(&mut self) -> Option<u32> {
            Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
        }
        fn list(&mut self) -> Option<Vec<u8>> {
            let n = self.word()? as usize;
            Some(self.take(n)?.to_vec())
        }
    }
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC {
        return None;
    }
    let stage = *STAGES.get(r.word()? as usize)?;
    let dxil = r.list()?;
    let n = r.word()? as usize;
    let mut user_data = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        user_data.push(UserData {
            index: r.word()?,
            slot: u8::try_from(r.word()?).ok()?,
            root: root_type(r.word()?),
            dword: r.word()?,
        });
    }
    let n = r.word()? as usize;
    let mut samplers = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        samplers.push([r.word()?, r.word()?, r.word()?, r.word()?]);
    }
    let n = r.word()? as usize;
    let mut static_samplers = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        static_samplers.push(StaticSampler {
            filter: r.word()?,
            address: [r.word()?, r.word()?, r.word()?],
            mip_lod_bias: f32::from_bits(r.word()?),
            max_anisotropy: r.word()?,
            comparison: r.word()?,
            border: r.word()?,
            min_lod: f32::from_bits(r.word()?),
            max_lod: f32::from_bits(r.word()?),
        });
    }
    let root_cbvs = r.list()?;
    let root_srvs = r.list()?;
    let root_uavs = r.list()?;
    let n = r.word()? as usize;
    let mut root_indexed_views = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        root_indexed_views.push(RootIndexedView {
            table_slot: u8::try_from(r.word()?).ok()?,
            constant_slot: u8::try_from(r.word()?).ok()?,
            constant_dword: r.word()?,
            byte_offset: r.word()?,
            shift: u8::try_from(r.word()?).ok().filter(|&s| s < 32)?,
        });
    }
    let n = r.word()? as usize;
    let mut half_views = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        half_views.push((u8::try_from(r.word()?).ok()?, r.word()?));
    }
    r.0.is_empty().then_some(HostShader {
        stage,
        dxil,
        user_data,
        samplers,
        static_samplers,
        root_cbvs,
        root_srvs,
        root_uavs,
        root_indexed_views,
        half_views,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader() -> HostShader {
        HostShader {
            stage: HwStage::Ps,
            dxil: b"DXBC and the rest".to_vec(),
            user_data: vec![UserData {
                index: 8,
                slot: 1,
                root: RootType::Table,
                dword: 0,
            }],
            samplers: vec![[0x400, 0xc0_0000, 0x18f0_0000, 0]],
            static_samplers: vec![StaticSampler::from_words([
                0x400,
                0xc0_0000,
                0x18f0_0000,
                0,
            ])],
            root_cbvs: vec![2],
            root_srvs: vec![0, 3],
            root_uavs: vec![1],
            root_indexed_views: vec![RootIndexedView {
                table_slot: 1,
                constant_slot: 2,
                constant_dword: 3,
                byte_offset: 32,
                shift: 5,
            }],
            half_views: vec![(0, 8), (4, 0)],
        }
    }

    #[test]
    fn translations_come_back_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        let build = [7u8; 32];
        let container = b"a console container".as_slice();
        let runs = AtomicU64::new(0);
        let translate = |_: &[u8]| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(shader())
        };
        assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        assert_eq!(runs.load(Ordering::Relaxed), 1, "the second came from disk");

        // Another build translates again.
        assert_eq!(cached(&cache, &[8; 32], container, translate), Ok(shader()));
        assert_eq!(runs.load(Ordering::Relaxed), 2);

        // A damaged entry is a miss, translated and stored again.
        for e in walk(dir.path()) {
            std::fs::write(e, b"VSH1 broken").unwrap();
        }
        assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        assert_eq!(runs.load(Ordering::Relaxed), 3);
        assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        assert_eq!(runs.load(Ordering::Relaxed), 3);

        // Failures are not stored.
        let fail = |_: &[u8]| Err::<HostShader, _>("unsupported".to_owned());
        assert!(cached(&cache, &build, b"other", fail).is_err());
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path().is_dir() {
                files.extend(walk(&e.path()));
            } else {
                files.push(e.path());
            }
        }
        files
    }

    #[test]
    fn a_reopened_cache_hits_again() {
        let dir = tempfile::tempdir().unwrap();
        let build = [9u8; 32];
        let container = b"console container".as_slice();
        let runs = AtomicU64::new(0);
        let translate = |_: &[u8]| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(shader())
        };
        {
            let cache = Cache::open(dir.path()).unwrap();
            assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        }
        {
            let cache = Cache::open(dir.path()).unwrap();
            assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        }
        assert_eq!(
            runs.load(Ordering::Relaxed),
            1,
            "the second process's open hit"
        );
    }

    /// Laptop switches do not change the translation, so they must not change
    /// the key. The facts struct has no field for them; two lookups with the
    /// same translator facts hit, which is what a second boot does when
    /// `VICTORY_PROFILE=small` and the texture and ambient switches are set.
    #[test]
    fn laptop_switches_do_not_spoil_a_hit() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        let facts = victory_shader::emit::TranslationFacts::current();
        let build = *blake3::hash(&victory_shader::emit::fingerprint_of(&facts)).as_bytes();
        let runs = AtomicU64::new(0);
        let translate = |_: &[u8]| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(shader())
        };
        let container = b"laptop boot container".as_slice();
        assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        assert_eq!(
            victory_shader::emit::fingerprint_of(&facts),
            victory_shader::emit::fingerprint_of(&victory_shader::emit::TranslationFacts::current())
        );
        assert_eq!(cached(&cache, &build, container, translate), Ok(shader()));
        assert_eq!(runs.load(Ordering::Relaxed), 1);
        assert!(
            !format!("{facts:?}").contains("PROFILE"),
            "the key's facts must not name a memory-profile switch"
        );
    }

    #[test]
    fn bytes_past_the_dxbc_container_are_not_a_different_shader() {
        let mut container = vec![0u8; 0x20];
        container[0..4].copy_from_slice(b"DXBC");
        container[0x18..0x1C].copy_from_slice(&0x20u32.to_le_bytes());
        let mut slack = container.clone();
        slack.extend_from_slice(&[0xA5; 64]);
        assert_eq!(shader_bytes(&container), shader_bytes(&slack));

        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        let build = [3u8; 32];
        let runs = AtomicU64::new(0);
        let translate = |_: &[u8]| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(shader())
        };
        assert_eq!(cached(&cache, &build, &container, translate), Ok(shader()));
        assert_eq!(cached(&cache, &build, &slack, translate), Ok(shader()));
        assert_eq!(
            runs.load(Ordering::Relaxed),
            1,
            "the tail is not part of the key"
        );
    }

    #[test]
    fn host_facts_survive_for_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        write_facts(dir.path(), 32, 999_999).unwrap();
        assert_eq!(read_facts(dir.path()), Some((32, 999_999)));
        std::fs::write(dir.path().join(FACTS_FILE), b"VHF0").unwrap();
        assert_eq!(read_facts(dir.path()), None);
    }

    #[test]
    fn entries_round_trip_and_reject_damage() {
        let bytes = encode(&shader());
        assert_eq!(decode(&bytes), Some(shader()));
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
        assert_eq!(decode(&[bytes.as_slice(), &[0]].concat()), None);
        assert_eq!(decode(b"VSH0"), None);
    }
}

//! Switches changed while the title runs, so one load into the world can
//! compare several settings: with `VICTORY_LIVE=<file>`, the file is read
//! again every second, and a switch made live ([`Switch`]) takes its value
//! from it, else from the environment as before. The file holds one
//! `NAME=value` a line (`#` starts a comment); `NAME=` with nothing after it
//! leaves the switch unset even when the environment sets it. The frame
//! line names the file's settings, and says when they changed during the
//! frames it sums ([`tag`]).
//!
//! Without `VICTORY_LIVE` nothing is read and a switch costs two atomic
//! loads, its value worked out once as before.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Once, OnceLock, RwLock};

/// The file's settings: a name with `None` is set to nothing.
type Values = BTreeMap<String, Option<String>>;

struct Live {
    path: PathBuf,
    values: RwLock<Values>,
    /// Bumped each time the file's settings change.
    generation: AtomicU64,
}

fn live() -> Option<&'static Live> {
    static LIVE: OnceLock<Option<Live>> = OnceLock::new();
    static WATCH: Once = Once::new();
    let live = LIVE
        .get_or_init(|| {
            let path = PathBuf::from(std::env::var_os("VICTORY_LIVE")?);
            let values = read(&path).unwrap_or_default();
            tracing::info!(file = %path.display(), "live switches: {}", text(&values));
            Some(Live { path, values: RwLock::new(values), generation: AtomicU64::new(1) })
        })
        .as_ref()?;
    WATCH.call_once(|| {
        let spawned = std::thread::Builder::new().name("live switches".into()).spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                // A file being written may read short: the next read sees it whole.
                let Some(values) = read(&live.path) else { continue };
                let mut now = live.values.write().unwrap_or_else(|e| e.into_inner());
                if *now != values {
                    tracing::info!("live switches: {}", text(&values));
                    *now = values;
                    live.generation.fetch_add(1, Ordering::AcqRel);
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "live switches not watched");
        }
    });
    Some(live)
}

fn read(path: &std::path::Path) -> Option<Values> {
    Some(parse(&std::fs::read_to_string(path).ok()?))
}

fn parse(text: &str) -> Values {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| {
            let value = value.trim();
            (name.trim().to_owned(), (!value.is_empty()).then(|| value.to_owned()))
        })
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

fn text(values: &Values) -> String {
    if values.is_empty() {
        return "none".into();
    }
    let all: Vec<String> =
        values.iter().map(|(name, value)| format!("{name}={}", value.as_deref().unwrap_or(""))).collect();
    all.join(" ")
}

/// How many times the settings changed: 0 without `VICTORY_LIVE`.
fn generation() -> u64 {
    live().map_or(0, |l| l.generation.load(Ordering::Acquire))
}

/// The setting of `name`: the file's when it names it, else the
/// environment's.
fn value(name: &str) -> Option<String> {
    if let Some(l) = live() {
        let values = l.values.read().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = values.get(name) {
            return value.clone();
        }
    }
    std::env::var(name).ok()
}

/// With `VICTORY_LIVE`: the settings, and whether they changed since the
/// last call (once a frame line: the line's frames are a mix).
pub(crate) fn tag() -> Option<String> {
    static TAGGED: AtomicU64 = AtomicU64::new(0);
    let l = live()?;
    let generation = l.generation.load(Ordering::Acquire);
    let before = TAGGED.swap(generation, Ordering::AcqRel);
    let values = text(&l.values.read().unwrap_or_else(|e| e.into_inner()));
    Some(if before == generation { values } else { format!("{values} (changed during these frames)") })
}

/// A switch read where it is used: its value is worked out from its
/// setting ([`value`]) again only after the settings change.
pub(crate) struct Switch {
    name: &'static str,
    parse: fn(Option<&str>) -> u32,
    seen: AtomicU64,
    value: AtomicU32,
}

impl Switch {
    pub(crate) const fn new(name: &'static str, parse: fn(Option<&str>) -> u32) -> Self {
        Self { name, parse, seen: AtomicU64::new(u64::MAX), value: AtomicU32::new(0) }
    }

    pub(crate) fn get(&self) -> u32 {
        let generation = generation();
        if self.seen.load(Ordering::Acquire) == generation {
            return self.value.load(Ordering::Relaxed);
        }
        let value = (self.parse)(value(self.name).as_deref());
        self.value.store(value, Ordering::Relaxed);
        self.seen.store(generation, Ordering::Release);
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_comments_and_unset_names() {
        let values = parse("A=1\n# B=2\n  C = x y # note\nD=\nnot a setting\n=3\n");
        assert_eq!(values.get("A"), Some(&Some("1".to_owned())));
        assert_eq!(values.get("B"), None);
        assert_eq!(values.get("C"), Some(&Some("x y".to_owned())));
        assert_eq!(values.get("D"), Some(&None));
        assert_eq!(values.len(), 3);
        assert_eq!(text(&values), "A=1 C=x y D=");
    }

    #[test]
    fn a_switch_without_the_file_reads_the_environment_once() {
        static S: Switch = Switch::new("VICTORY_LIVE_TEST_UNSET", |v| u32::from(v.is_some()) + 7);
        assert_eq!(S.get(), 7);
        assert_eq!(S.get(), 7);
    }
}

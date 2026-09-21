//! Ce qui mesure, et non ce qui splitte.
//!
//! Deux choses seulement sont du metier et vivent donc dans toutes les
//! compilations : l'horloge, et [`FreshMap`] -- le correctif du cache d'une
//! seconde en depend. Tout le reste ne sert qu'a mesurer et disparait sans la
//! feature `diagnostics` : [`ScanTrace`] devient un type vide, [`event`] et
//! [`validation_read`] des fonctions sans corps.
//!
//! La regle qui va avec : le code metier n'imprime rien pour mesurer. Il
//! appelle d'ici, et c'est la feature qui decide s'il se passe quelque chose.

#[cfg(feature = "diagnostics")]
use core::sync::atomic::{AtomicU64, Ordering};

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn clock_time_get(id: u32, precision: u64, result: *mut u64) -> u16;
}

pub fn now_us() -> u64 {
    let mut ns = 0;
    // WASI CLOCKID_MONOTONIC. Le runtime ecrit un entier u64 dans `ns`.
    let error = unsafe { clock_time_get(1, 1, &mut ns) };
    assert_eq!(error, 0, "horloge WASI indisponible");
    ns / 1000
}

#[cfg(feature = "diagnostics")]
static CALLS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "diagnostics")]
static BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "diagnostics")]
static FAILURES: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn validation_read(_bytes: usize, _ok: bool) {
    #[cfg(feature = "diagnostics")]
    {
        CALLS.fetch_add(1, Ordering::Relaxed);
        if _ok { BYTES.fetch_add(_bytes as u64, Ordering::Relaxed); }
        else { FAILURES.fetch_add(1, Ordering::Relaxed); }
    }
}

/// Lectures unitaires faites hors balayage -- validation des candidats.
///
/// Elles ne passent pas par le budget du balayage, donc elles lui echappent :
/// les compter a part est la seule facon de savoir si le temps part dans les
/// passes ou dans les allers-retours qui les suivent.
#[cfg(feature = "diagnostics")]
fn validation_counts() -> [u64; 3] {
    [
        CALLS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
        FAILURES.load(Ordering::Relaxed),
    ]
}

/// Un depart, et son contexte : premier du module, premier de ce processus,
/// ou enieme relance. C'est ce que `summarize_startup.py` classe -- un module
/// fraichement charge n'a aucun cache, un processus neuf en a d'autres.
///
/// L'etat vit ici et non dans la boucle : sans cela, deux variables
/// traversaient la signature de `run` pour alimenter une seule trace.
#[cfg(feature = "diagnostics")]
static STARTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "diagnostics")]
static LAST_PID: AtomicU64 = AtomicU64::new(u64::MAX);

pub fn started(_elapsed_ms: i64, _pid: asr::ProcessId) {
    #[cfg(feature = "diagnostics")]
    {
        let first_module = STARTS.fetch_add(1, Ordering::Relaxed) == 0;
        let first_process = LAST_PID.swap(_pid.0, Ordering::Relaxed) != _pid.0;
        asr::print_message(&alloc::format!(
            "HF_START elapsed_ms={_elapsed_ms} first_module={first_module} first_process={first_process}"
        ));
    }
}

pub fn event(_name: &str) {
    #[cfg(feature = "diagnostics")]
    asr::print_message(&alloc::format!("HF_DIAG event={} t_us={}", _name, now_us()));
}

/// Carte obtenue par un nouvel acces au meme processus, toutes les 100 ms.
/// L'acces principal et les ancres de la partie restent valides.
#[derive(Default)]
pub struct FreshMap {
    next_us: u64,
    ranges: Option<alloc::vec::Vec<(u64, u64)>>,
}

impl FreshMap {
    pub fn poll(&mut self, pid: asr::ProcessId) -> Option<&[(u64, u64)]> {
        let now = now_us();
        if now >= self.next_us {
            self.next_us = now + 100_000;
            if let Some(process) = asr::Process::attach_by_pid(pid) {
                let ranges = crate::hammerfest::heap_ranges(&process);
                #[cfg(feature = "diagnostics")]
                asr::print_message(&alloc::format!(
                    "HF_DIAG event=map_refresh t_us={} elapsed_us={} changed={} ranges={} bytes={}",
                    now, now_us() - now, self.ranges.as_ref() != Some(&ranges), ranges.len(),
                    ranges.iter().map(|(a,b)| b-a).sum::<u64>()
                ));
                self.ranges = Some(ranges);
            } else {
                self.ranges = None;
                event("map_refresh_failed");
            }
        }
        self.ranges.as_deref()
    }
}


/// Ce qu'une tentative de resolution a coute, et ou elle en etait.
///
/// Sans la feature, ce type ne contient rien et toutes ses methodes sont
/// vides : le balayage garde ses appels, le binaire n'en garde aucun.
#[cfg(feature = "diagnostics")]
pub struct ScanTrace {
    bytes: u64,
    failures: u64,
    yields: u32,
    started: u64,
    stage_started: u64,
    stage: &'static str,
    outcome: &'static str,
    validation: [u64; 3],
}

#[cfg(not(feature = "diagnostics"))]
pub struct ScanTrace;

#[cfg(feature = "diagnostics")]
impl Default for ScanTrace {
    fn default() -> Self {
        let now = now_us();
        Self {
            bytes: 0,
            failures: 0,
            yields: 0,
            started: now,
            stage_started: now,
            stage: "ranges",
            outcome: "not_found",
            validation: validation_counts(),
        }
    }
}

#[cfg(not(feature = "diagnostics"))]
impl Default for ScanTrace {
    fn default() -> Self {
        Self
    }
}

#[cfg(feature = "diagnostics")]
impl ScanTrace {
    pub fn read(&mut self, bytes: usize, ok: bool) {
        if ok {
            self.bytes += bytes as u64;
        } else {
            self.failures += 1;
        }
    }

    pub fn paused(&mut self) {
        self.yields += 1;
    }

    pub fn stage(&mut self, next: &'static str, requested: u64, calls: u64) {
        let now = now_us();
        asr::print_message(&alloc::format!(
            "HF_DIAG event=stage t_us={now} name={} elapsed_us={} scan_bytes_total={} scan_calls_total={calls}",
            self.stage,
            now - self.stage_started,
            requested
        ));
        self.stage = next;
        self.stage_started = now;
    }

    pub fn outcome(&mut self, outcome: &'static str) {
        self.outcome = outcome;
    }

    /// Le bilan, a la destruction : la tentative est finie, quel que soit le
    /// chemin de sortie -- et il y en a six.
    pub fn finish(&mut self, requested: u64, calls: u64) {
        let now = now_us();
        let validation = validation_counts();
        asr::print_message(&alloc::format!(
            "HF_SCAN t_us={now} outcome={} elapsed_us={} requested_bytes={requested} read_bytes={} calls={calls} failures={} yields={} validation_calls={} validation_bytes={} validation_failures={}",
            self.outcome,
            now - self.started,
            self.bytes,
            self.failures,
            self.yields,
            validation[0] - self.validation[0],
            validation[1] - self.validation[1],
            validation[2] - self.validation[2]
        ));
    }
}

#[cfg(not(feature = "diagnostics"))]
impl ScanTrace {
    #[inline]
    pub fn read(&mut self, _bytes: usize, _ok: bool) {}
    #[inline]
    pub fn paused(&mut self) {}
    #[inline]
    pub fn stage(&mut self, _next: &'static str, _requested: u64, _calls: u64) {}
    #[inline]
    pub fn outcome(&mut self, _outcome: &'static str) {}
    #[inline]
    pub fn finish(&mut self, _requested: u64, _calls: u64) {}
}

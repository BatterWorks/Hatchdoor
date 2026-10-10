use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::Serialize;

use crate::vault_registry::VaultId;
use crate::vault_runtime::{VaultPhase, VaultRuntime, VaultSource};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct IndexingProgressSnapshot {
    pub notes_completed: usize,
    pub notes_total: usize,
    pub chunks_completed: usize,
    pub chunks_total: usize,
    pub tokens_completed: usize,
    pub tokens_total: usize,
    pub elapsed_seconds: u64,
}

impl IndexingProgressSnapshot {
    fn percent(self) -> u8 {
        if self.tokens_total == 0 {
            return 0;
        }
        ((self.tokens_completed.saturating_mul(100) / self.tokens_total).min(100)) as u8
    }

    fn eta_seconds(self) -> Option<u64> {
        if self.tokens_completed == 0 || self.tokens_completed >= self.tokens_total {
            return None;
        }
        let remaining = self.tokens_total - self.tokens_completed;
        Some(self.elapsed_seconds.saturating_mul(remaining as u64) / self.tokens_completed as u64)
    }
}

/// One active Vault's place in a first-run indexing pass, as the Vault
/// collection reports it when an Index turn speaks. `settled` follows the
/// same rule as the collection's readiness check: ready, stale, failed, or no
/// longer active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexingParticipant {
    pub vault_id: VaultId,
    pub settled: bool,
}

/// What one first-run indexing pass knows about every Vault in it, so the
/// startup reading can describe the whole job instead of whichever Vault
/// spoke last (#373). Index turns run one at a time, so without this the
/// percent climbed to 100 for each Vault and dropped back to 0 for the next.
///
/// In memory only, like the rest of this tracker. Model setup starting the
/// collection over begins a new pass.
#[derive(Debug, Default)]
struct FirstRunPass {
    /// Bumped on every reset, so a note count started for an earlier pass
    /// cannot land in this one.
    generation: u64,
    note_counts_claimed: bool,
    /// The active Vaults as of the latest report. `None` until a Vault turn
    /// has reported.
    participants: Option<Vec<IndexingParticipant>>,
    /// Each Vault's latest Index progress in this pass.
    reported: BTreeMap<VaultId, IndexingProgressSnapshot>,
    /// Queued Vaults' approximate note counts; `None` when the count failed.
    note_counts: BTreeMap<VaultId, Option<usize>>,
    /// The highest percent reported this pass. A queued Vault's estimate
    /// can be corrected upward once its real token total is known, and the
    /// reading must not move backwards when it is.
    high_water: u8,
}

impl FirstRunPass {
    fn reset(&mut self) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            ..Self::default()
        };
    }

    /// The collection-wide percent and time left, given the progress of the
    /// Vault currently building.
    fn reading(&mut self, current: IndexingProgressSnapshot) -> (u8, Option<u64>) {
        let single = (current.percent(), current.eta_seconds());
        let Some(participants) = &self.participants else {
            return single;
        };
        let vaults: BTreeSet<VaultId> = participants
            .iter()
            .map(|participant| participant.vault_id)
            .chain(self.reported.keys().copied())
            .collect();
        if vaults.len() <= 1 {
            // A Vault that left the active set must not take the reading
            // back below what it showed; with only ever one Vault the mark
            // stays 0 and this is that Vault's own reading.
            return (single.0.max(self.high_water), single.1);
        }
        let settled = |vault_id: &VaultId| {
            participants
                .iter()
                .find(|participant| participant.vault_id == *vault_id)
                .is_none_or(|participant| participant.settled)
        };

        // A queued Vault has no token count until its own turn chunks it,
        // so it is weighted by its note count times the tokens per note of
        // the Vaults whose real totals are known.
        let known_notes: Vec<usize> = vaults
            .iter()
            .filter_map(|vault_id| self.note_count(vault_id))
            .collect();
        let average_notes = (!known_notes.is_empty())
            .then(|| known_notes.iter().sum::<usize>() / known_notes.len());
        let (sample_tokens, sample_notes) = self
            .reported
            .values()
            .filter(|progress| progress.notes_total > 0)
            .fold((0u64, 0u64), |(tokens, notes), progress| {
                (
                    tokens.saturating_add(progress.tokens_total as u64),
                    notes.saturating_add(progress.notes_total as u64),
                )
            });

        let mut total = 0u64;
        let mut done = 0u64;
        for vault_id in &vaults {
            let is_settled = settled(vault_id);
            match self.reported.get(vault_id) {
                Some(progress) => {
                    let weight = progress.tokens_total as u64;
                    total = total.saturating_add(weight);
                    done = done.saturating_add(if is_settled {
                        weight
                    } else {
                        (progress.tokens_completed as u64).min(weight)
                    });
                }
                // Settled without reporting: it had nothing to embed, or it
                // failed before embedding. Either way none of the job's work
                // is waiting on it.
                None if is_settled => {}
                None => {
                    let notes = self.note_count(vault_id).or(average_notes);
                    let (Some(notes), true) = (notes, sample_notes > 0) else {
                        // Nothing to estimate it from yet: hold the reading.
                        return (self.high_water, None);
                    };
                    total = total.saturating_add(
                        (notes as u64).saturating_mul(sample_tokens) / sample_notes,
                    );
                }
            }
        }

        let computed = done
            .saturating_mul(100)
            .checked_div(total)
            .map_or(0, |percent| percent.min(100) as u8);
        self.high_water = self.high_water.max(computed);

        let (completed, elapsed) =
            self.reported
                .values()
                .fold((0u64, 0u64), |(completed, elapsed), progress| {
                    (
                        completed.saturating_add(progress.tokens_completed as u64),
                        elapsed.saturating_add(progress.elapsed_seconds),
                    )
                });
        let remaining = total.saturating_sub(done);
        let eta =
            (completed > 0 && remaining > 0).then(|| elapsed.saturating_mul(remaining) / completed);
        (self.high_water, eta)
    }

    /// A Vault's note count: its own turn's figure once it has reported,
    /// otherwise the approximate count taken while it waited.
    fn note_count(&self, vault_id: &VaultId) -> Option<usize> {
        self.reported
            .get(vault_id)
            .map(|progress| progress.notes_total)
            .filter(|notes| *notes > 0)
            .or_else(|| self.note_counts.get(vault_id).copied().flatten())
    }
}

/// The code [`StartupTracker::set_model_setup_failed`] publishes. It is what
/// tells a failed model setup apart from every other reason this tracker can
/// be `Unavailable`, which is the distinction
/// [`StartupTracker::model_setup_pending`] turns on.
const MODEL_SETUP_FAILED: &str = "model_setup_failed";

#[derive(Clone, Debug)]
pub struct StartupTracker {
    runtime: VaultRuntime,
    pass: Arc<Mutex<FirstRunPass>>,
}

impl StartupTracker {
    pub fn new(runtime: VaultRuntime) -> Self {
        Self {
            runtime,
            pass: Arc::default(),
        }
    }

    pub fn terms_required() -> Self {
        let runtime = VaultRuntime::new(VaultSource::Local {
            vault_path: "./vault".into(),
        });
        runtime.set_terms_required();
        Self::new(runtime)
    }

    pub fn scanning() -> Self {
        let runtime = VaultRuntime::new(VaultSource::Local {
            vault_path: "./vault".into(),
        });
        runtime.set_scanning();
        Self::new(runtime)
    }

    pub fn ready() -> Self {
        Self::new(VaultRuntime::ready(VaultSource::Local {
            vault_path: "./vault".into(),
        }))
    }

    fn pass(&self) -> std::sync::MutexGuard<'_, FirstRunPass> {
        self.pass
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Model setup starting the collection over starts the indexing pass
    /// over with it: the next Index turn's progress begins a new reading.
    fn restart_pass(&self) {
        self.pass().reset();
    }

    pub fn set_scanning(&self) {
        self.restart_pass();
        self.runtime.set_scanning();
    }

    pub fn set_terms_required(&self) {
        self.restart_pass();
        self.runtime.set_terms_required();
    }

    pub fn set_downloading(
        &self,
        model: &'static str,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
    ) {
        self.restart_pass();
        self.runtime
            .set_downloading(model, downloaded_bytes, total_bytes);
    }

    /// Move the tracker to `Indexing` even when it has settled `Ready`, for
    /// fixtures that need an instance still indexing. Test-only so production
    /// code can report progress only through [`Self::report_indexing_progress`]
    /// and its latch.
    #[cfg(test)]
    pub(crate) fn set_indexing(&self, progress: IndexingProgressSnapshot) {
        self.runtime.set_indexing(progress);
    }

    /// Report an Index turn's progress, unless the collection has already
    /// settled `Ready`. Once it has, a later Index turn is routine upkeep of
    /// one Vault, which that Vault reports itself; moving this tracker back to
    /// `Indexing` for it took the whole instance out of readiness (`/ready`
    /// answered 503) for the length of every watcher-triggered embedding pass
    /// (#326). Only model setup, which starts the collection over, leaves
    /// `Ready` again.
    ///
    /// `vault_id` is the Vault the progress belongs to, and `participants`
    /// every active Vault with its settled state, so the reading can cover
    /// the whole first-run job rather than this one Vault (#373).
    pub fn report_indexing_progress(
        &self,
        vault_id: VaultId,
        progress: IndexingProgressSnapshot,
        participants: Vec<IndexingParticipant>,
    ) {
        if self.runtime.is_ready() {
            return;
        }
        {
            let mut pass = self.pass();
            pass.reported.insert(vault_id, progress);
            pass.participants = Some(participants);
        }
        self.runtime.set_indexing_unless_ready(progress);
    }

    /// Replace the participants' settled state without new progress, for a
    /// turn that has just finished: a Vault that failed partway counts as
    /// done from then on, not from the next Vault's first report.
    pub fn refresh_indexing_participants(&self, participants: Vec<IndexingParticipant>) {
        let mut pass = self.pass();
        if pass.participants.is_some() {
            pass.participants = Some(participants);
        }
    }

    /// Claim this pass's one round of note counting for queued Vaults.
    /// Returns the pass generation the counts must be recorded against, or
    /// `None` when the round is already claimed or the collection is ready.
    pub fn claim_note_counts(&self) -> Option<u64> {
        if self.runtime.is_ready() {
            return None;
        }
        let mut pass = self.pass();
        if pass.note_counts_claimed {
            return None;
        }
        pass.note_counts_claimed = true;
        Some(pass.generation)
    }

    /// A queued Vault's recorded note count in the current pass, for tests
    /// that need to see a background count land.
    #[cfg(test)]
    pub(crate) fn recorded_note_count(&self, vault_id: VaultId) -> Option<Option<usize>> {
        self.pass().note_counts.get(&vault_id).copied()
    }

    /// Record a queued Vault's approximate note count, or `None` when its
    /// directory could not be counted. Ignored if the pass has restarted
    /// since `generation` was claimed.
    pub fn record_note_count(&self, generation: u64, vault_id: VaultId, notes: Option<usize>) {
        let mut pass = self.pass();
        if pass.generation == generation {
            pass.note_counts.insert(vault_id, notes);
        }
    }

    pub fn set_ready(&self) {
        self.runtime.set_ready();
    }

    /// Latch `Ready` unless model setup stands in the way (terms
    /// outstanding, a download in flight, a failed setup) or it is latched
    /// already. Returns whether this call latched it.
    pub fn settle_ready(&self) -> bool {
        self.runtime.settle_ready()
    }

    pub fn set_model_setup_failed(&self) {
        self.restart_pass();
        self.runtime.set_unavailable(
            MODEL_SETUP_FAILED,
            "The search model could not be downloaded or loaded. Check the Hatchdoor logs, then retry setup.",
        );
    }

    /// Whether the model is set up and every active Vault's Index turn has
    /// settled, which is the condition `vault_executor::settle_startup`
    /// latches here through `collection_indexes_settled`. An instance with no
    /// active Vault has nothing to settle (#453). Once latched it stays true
    /// through later rebuilds and single-Vault failures; only model setup
    /// resets it.
    ///
    /// Named for what it measures rather than for `Ready`, because the shorter
    /// `is_ready` invited a question it cannot answer: three callers read it as
    /// "has first-run setup finished", and so reported a routine post-write
    /// reindex as incomplete setup (#191). Ask
    /// [`Self::model_setup_pending`] for that.
    pub fn collection_indexes_ready(&self) -> bool {
        self.runtime.is_ready()
    }

    /// Whether first-run model setup is genuinely what stands between a caller
    /// and the Vault collection.
    ///
    /// Deliberately not the negation of [`Self::collection_indexes_ready`].
    /// This tracker's phase carries the first-run setup lifecycle and the
    /// first-run indexing progress `VaultWorkExecutor` reports. Once the
    /// collection settles `Ready`, [`Self::report_indexing_progress`] drops
    /// further progress, so a routine reindex no longer moves the phase; it is
    /// reported per Vault through `VaultSearchStatus`, and only model setup
    /// leaves `Ready` again. Before that, a Vault still in its first index, or
    /// an `Unavailable` that is not a setup failure, is neither ready nor a
    /// setup problem. Reading "not ready" as a setup answer is what #191 was.
    ///
    /// Only a pending terms choice, a download in flight, and a failed setup
    /// are conditions the setup tools can act on. Validating, scanning and
    /// indexing are not: each Vault reports those itself, per Vault and
    /// accurately, through its own `VaultSearchStatus`.
    pub fn model_setup_pending(&self) -> bool {
        let snapshot = self.runtime.snapshot();
        match snapshot.phase {
            VaultPhase::TermsRequired | VaultPhase::Downloading => true,
            VaultPhase::Unavailable => snapshot
                .error
                .is_some_and(|error| error.code == MODEL_SETUP_FAILED),
            VaultPhase::Validating
            | VaultPhase::Scanning
            | VaultPhase::Indexing
            | VaultPhase::Ready => false,
        }
    }

    pub fn runtime(&self) -> &VaultRuntime {
        &self.runtime
    }

    pub fn status(&self) -> StartupStatusResponse {
        let snapshot = self.runtime.snapshot();
        match snapshot.phase {
            VaultPhase::TermsRequired => StartupStatusResponse::simple("terms_required", None),
            VaultPhase::Downloading => StartupStatusResponse {
                state: "downloading",
                model: snapshot.model,
                downloaded_bytes: snapshot.downloaded_bytes,
                total_bytes: snapshot.total_bytes,
                notes_completed: None,
                notes_total: None,
                chunks_completed: None,
                chunks_total: None,
                tokens_completed: None,
                tokens_total: None,
                percent: download_percent(snapshot.downloaded_bytes, snapshot.total_bytes),
                eta_seconds: None,
                message: None,
            },
            VaultPhase::Validating | VaultPhase::Scanning => {
                StartupStatusResponse::simple("scanning", None)
            }
            VaultPhase::Indexing => {
                let progress = snapshot.indexing.unwrap_or_default();
                let (percent, eta_seconds) = self.pass().reading(progress);
                StartupStatusResponse {
                    state: "indexing",
                    model: None,
                    downloaded_bytes: None,
                    total_bytes: None,
                    notes_completed: Some(progress.notes_completed),
                    notes_total: Some(progress.notes_total),
                    chunks_completed: Some(progress.chunks_completed),
                    chunks_total: Some(progress.chunks_total),
                    tokens_completed: Some(progress.tokens_completed),
                    tokens_total: Some(progress.tokens_total),
                    percent: Some(percent),
                    eta_seconds,
                    message: None,
                }
            }
            VaultPhase::Ready => StartupStatusResponse::simple("ready", None),
            VaultPhase::Unavailable => {
                StartupStatusResponse::simple("failed", snapshot.error.map(|error| error.message))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct StartupStatusResponse {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    /// The `notes_*`, `chunks_*` and `tokens_*` counters describe the Vault
    /// currently indexing, not the whole collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_total: Option<usize>,
    /// While indexing, how far the whole first-run job has got across every
    /// active Vault. It never decreases within one pass. With one Vault it is
    /// that Vault's own percent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    /// While indexing, the estimated seconds left for the whole first-run
    /// job, queued Vaults included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl StartupStatusResponse {
    fn simple(state: &'static str, message: Option<String>) -> Self {
        Self {
            state,
            model: None,
            downloaded_bytes: None,
            total_bytes: None,
            notes_completed: None,
            notes_total: None,
            chunks_completed: None,
            chunks_total: None,
            tokens_completed: None,
            tokens_total: None,
            percent: None,
            eta_seconds: None,
            message,
        }
    }
}

fn download_percent(downloaded: Option<u64>, total: Option<u64>) -> Option<u8> {
    let (Some(downloaded), Some(total)) = (downloaded, total) else {
        return None;
    };
    (total > 0).then(|| ((downloaded.saturating_mul(100) / total).min(100)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_required_is_not_ready_and_is_exposed_to_the_ui() {
        let tracker = StartupTracker::terms_required();
        assert!(!tracker.collection_indexes_ready());
        let status = tracker.status();
        assert_eq!(status.state, "terms_required");
        assert!(status.percent.is_none());
    }

    #[test]
    fn download_status_carries_model_and_byte_progress() {
        let tracker = StartupTracker::terms_required();
        tracker.set_downloading("EmbeddingGemma 300M Q4", Some(25), Some(100));
        let status = tracker.status();
        assert_eq!(status.state, "downloading");
        assert_eq!(status.model, Some("EmbeddingGemma 300M Q4"));
        assert_eq!(status.downloaded_bytes, Some(25));
        assert_eq!(status.total_bytes, Some(100));
        assert_eq!(status.percent, Some(25));
    }

    #[test]
    fn unknown_download_size_does_not_invent_a_percentage() {
        let tracker = StartupTracker::terms_required();
        tracker.set_downloading("Nomic Embed Text v1.5", None, None);
        assert_eq!(tracker.status().percent, None);
    }

    /// Setup is pending only while the setup tools can still change something:
    /// a terms choice is outstanding, a download is in flight, or a setup
    /// failed and can be retried.
    #[test]
    fn only_the_setup_phases_report_setup_as_pending() {
        let tracker = StartupTracker::terms_required();
        assert!(tracker.model_setup_pending());

        tracker.set_downloading("EmbeddingGemma 300M Q4", Some(1), Some(2));
        assert!(tracker.model_setup_pending());

        tracker.set_model_setup_failed();
        assert!(tracker.model_setup_pending());
    }

    /// A collection that is rebuilding has finished setup, so the setup tools
    /// have nothing to offer it. This is #191: a post-write Index turn reports
    /// its progress on this very tracker, and treating that as `!is_ready()`
    /// sent every MCP caller to the model-setup tools for the duration.
    #[test]
    fn rebuilding_is_not_pending_setup() {
        let tracker = StartupTracker::ready();
        assert!(!tracker.model_setup_pending());

        tracker.set_scanning();
        assert!(!tracker.model_setup_pending());
        assert!(
            !tracker.collection_indexes_ready(),
            "still not Ready, just not a setup problem"
        );

        tracker.set_indexing(IndexingProgressSnapshot::default());
        assert!(!tracker.model_setup_pending());
    }

    /// A settled collection stays ready through a later Index turn's
    /// progress reports: that turn is one Vault's upkeep, not startup (#326).
    #[test]
    fn indexing_progress_after_readiness_does_not_leave_ready() {
        let tracker = StartupTracker::scanning();
        let vault = vault_id();
        tracker.report_indexing_progress(
            vault,
            IndexingProgressSnapshot::default(),
            vec![participant(vault, false)],
        );
        assert_eq!(
            tracker.status().state,
            "indexing",
            "first-run progress is still reported"
        );

        tracker.set_ready();
        tracker.report_indexing_progress(
            vault,
            IndexingProgressSnapshot::default(),
            vec![participant(vault, false)],
        );
        assert!(tracker.collection_indexes_ready());
        assert_eq!(tracker.status().state, "ready");
    }

    /// `Unavailable` is not one condition: a failed index and a registry
    /// awaiting operator recovery both land here, and neither is answered by
    /// accepting a model licence. Only the error code tells them apart.
    #[test]
    fn unavailable_for_a_non_setup_reason_is_not_pending_setup() {
        let tracker = StartupTracker::ready();
        tracker
            .runtime()
            .set_unavailable("vault_index_failed", "Indexing could not be completed.");
        assert!(!tracker.model_setup_pending());

        tracker.runtime().set_unavailable(
            "startup_recovery_required",
            "Startup recovery is required before Vaults can be activated",
        );
        assert!(!tracker.model_setup_pending());
    }

    fn vault_id() -> VaultId {
        VaultId::generate().expect("vault id")
    }

    fn participant(vault_id: VaultId, settled: bool) -> IndexingParticipant {
        IndexingParticipant { vault_id, settled }
    }

    fn progress(
        notes_total: usize,
        tokens_completed: usize,
        tokens_total: usize,
        elapsed_seconds: u64,
    ) -> IndexingProgressSnapshot {
        IndexingProgressSnapshot {
            notes_completed: 0,
            notes_total,
            chunks_completed: 0,
            chunks_total: 0,
            tokens_completed,
            tokens_total,
            elapsed_seconds,
        }
    }

    /// Drives a first-run pass over `vaults`, one Vault after another the
    /// way the serialized Index turns run them, and returns every reading.
    /// Each entry is the Vault's real token total; every Vault has 10 notes,
    /// and the queued Vaults' counts are recorded before the first report.
    fn run_pass(tracker: &StartupTracker, vaults: &[(VaultId, usize)]) -> Vec<(u8, Option<u64>)> {
        let generation = tracker.claim_note_counts().expect("first claim");
        for (vault_id, _) in vaults {
            tracker.record_note_count(generation, *vault_id, Some(10));
        }
        let mut readings = Vec::new();
        for (index, (vault_id, tokens)) in vaults.iter().enumerate() {
            let participants = |current_settled: bool| {
                vaults
                    .iter()
                    .enumerate()
                    .map(|(other, (id, _))| {
                        participant(*id, other < index || (other == index && current_settled))
                    })
                    .collect::<Vec<_>>()
            };
            for step in 0..=4 {
                tracker.report_indexing_progress(
                    *vault_id,
                    progress(10, tokens * step / 4, *tokens, step as u64 * 10),
                    participants(false),
                );
                let status = tracker.status();
                readings.push((status.percent.expect("percent"), status.eta_seconds));
            }
            tracker.refresh_indexing_participants(participants(true));
            let status = tracker.status();
            readings.push((status.percent.expect("percent"), status.eta_seconds));
        }
        readings
    }

    fn assert_never_decreases(readings: &[(u8, Option<u64>)]) {
        for pair in readings.windows(2) {
            assert!(
                pair[1].0 >= pair[0].0,
                "percent went backwards: {readings:?}"
            );
        }
    }

    /// Three Vaults indexed one after another: the reading covers all three,
    /// so it never drops back to 0 when the next Vault's turn starts (#373).
    #[test]
    fn first_run_percent_covers_every_vault_and_never_decreases() {
        let tracker = StartupTracker::scanning();
        let vaults = [
            (vault_id(), 1_000),
            (vault_id(), 1_000),
            (vault_id(), 1_000),
        ];
        let readings = run_pass(&tracker, &vaults);

        assert_never_decreases(&readings);
        assert_eq!(readings[0].0, 0);
        // The first Vault's turn finished: a third of the job.
        assert_eq!(readings[5].0, 33);
        assert_eq!(readings.last().expect("reading").0, 100);

        tracker.set_ready();
        assert_eq!(tracker.status().state, "ready");
    }

    /// The time left after the first Vault counts the Vaults still queued,
    /// and is known from the first report of each later Vault.
    #[test]
    fn first_run_eta_counts_the_queued_vaults() {
        let tracker = StartupTracker::scanning();
        let first = vault_id();
        let second = vault_id();
        let third = vault_id();
        let all = |settled: [bool; 3]| {
            vec![
                participant(first, settled[0]),
                participant(second, settled[1]),
                participant(third, settled[2]),
            ]
        };
        let generation = tracker.claim_note_counts().expect("claim");
        for vault in [first, second, third] {
            tracker.record_note_count(generation, vault, Some(10));
        }
        tracker.report_indexing_progress(first, progress(10, 1_000, 1_000, 100), all([false; 3]));
        tracker.refresh_indexing_participants(all([true, false, false]));

        let started = progress(10, 0, 1_000, 0);
        tracker.report_indexing_progress(second, started, all([true, false, false]));
        assert_eq!(started.eta_seconds(), None, "alone it knows no throughput");
        assert_eq!(
            tracker.status().eta_seconds,
            Some(200),
            "two Vaults of work at the first Vault's 10 tokens a second"
        );

        let halfway = progress(10, 500, 1_000, 50);
        tracker.report_indexing_progress(second, halfway, all([true, false, false]));
        let eta = tracker.status().eta_seconds.expect("eta");
        assert!(
            eta > halfway.eta_seconds().expect("own eta"),
            "{eta} must cover the queued Vault too"
        );
        assert_eq!(eta, 150);
    }

    /// A queued Vault's real token total can be far from its note-count
    /// estimate either way; correcting it never moves the reading back.
    #[test]
    fn a_corrected_estimate_never_moves_the_percent_back() {
        for real_tokens in [50, 20_000] {
            let tracker = StartupTracker::scanning();
            let readings = run_pass(&tracker, &[(vault_id(), 1_000), (vault_id(), real_tokens)]);
            assert_never_decreases(&readings);
            assert_eq!(readings.last().expect("reading").0, 100);
        }
    }

    /// With one Vault the reading is exactly that Vault's own.
    #[test]
    fn a_single_vault_reading_is_unchanged() {
        let tracker = StartupTracker::scanning();
        let vault = vault_id();
        for snapshot in [
            progress(10, 0, 1_000, 0),
            progress(10, 250, 1_000, 30),
            progress(10, 999, 1_000, 120),
            progress(10, 1_000, 1_000, 121),
            progress(0, 0, 0, 0),
        ] {
            tracker.report_indexing_progress(vault, snapshot, vec![participant(vault, false)]);
            let status = tracker.status();
            assert_eq!(status.percent, Some(snapshot.percent()));
            assert_eq!(status.eta_seconds, snapshot.eta_seconds());
        }
    }

    /// A Vault that fails partway, and one disabled before its turn, both
    /// stop holding the reading back: it still reaches 100.
    #[test]
    fn failed_and_disabled_vaults_count_as_done() {
        let tracker = StartupTracker::scanning();
        let failed = vault_id();
        let disabled = vault_id();
        let last = vault_id();
        let generation = tracker.claim_note_counts().expect("claim");
        for vault in [failed, disabled, last] {
            tracker.record_note_count(generation, vault, Some(10));
        }
        tracker.report_indexing_progress(
            failed,
            progress(10, 300, 1_000, 30),
            vec![
                participant(failed, false),
                participant(disabled, false),
                participant(last, false),
            ],
        );
        // The turn failed: its Vault settled with the failure as its status.
        // Then the second Vault is disabled, leaving the active set.
        tracker.refresh_indexing_participants(vec![
            participant(failed, true),
            participant(disabled, false),
            participant(last, false),
        ]);
        let before = tracker.status().percent.expect("percent");
        assert_eq!(before, 33, "the failed Vault's whole share is done");

        tracker.report_indexing_progress(
            last,
            progress(10, 1_000, 1_000, 100),
            vec![participant(failed, true), participant(last, false)],
        );
        assert_eq!(tracker.status().percent, Some(100));
    }

    /// Model setup starting over starts the reading over: the next pass
    /// begins from 0, and a note count from the old pass is dropped.
    #[test]
    fn restarting_model_setup_resets_the_pass() {
        let tracker = StartupTracker::scanning();
        let first = vault_id();
        let second = vault_id();
        let stale_generation = tracker.claim_note_counts().expect("claim");
        tracker.report_indexing_progress(
            first,
            progress(10, 900, 1_000, 90),
            vec![participant(first, false), participant(second, false)],
        );
        tracker.record_note_count(stale_generation, second, Some(10));
        assert_eq!(tracker.status().percent, Some(45));

        tracker.set_downloading("EmbeddingGemma 300M Q4", Some(1), Some(2));
        tracker.set_scanning();
        assert!(
            tracker.claim_note_counts().is_some(),
            "a new pass counts its queued Vaults again"
        );
        tracker.record_note_count(stale_generation, second, Some(1_000_000));
        tracker.report_indexing_progress(
            first,
            progress(10, 0, 1_000, 0),
            vec![participant(first, false), participant(second, false)],
        );
        // The old 45% high-water mark is gone, and the stale million-note
        // count did not land: the second Vault is weighted like the first.
        assert_eq!(tracker.status().percent, Some(0));
        tracker.report_indexing_progress(
            first,
            progress(10, 500, 1_000, 10),
            vec![participant(first, false), participant(second, false)],
        );
        assert_eq!(tracker.status().percent, Some(25));
    }

    /// After `Ready`, routine reindex progress is not recorded either, so it
    /// cannot leak into the reading of a later setup pass.
    #[test]
    fn progress_after_ready_is_not_recorded() {
        let tracker = StartupTracker::ready();
        let vault = vault_id();
        tracker.report_indexing_progress(
            vault,
            progress(10, 5, 10, 1),
            vec![participant(vault, false)],
        );
        assert!(tracker.pass().reported.is_empty());
        assert_eq!(tracker.claim_note_counts(), None);
    }
}

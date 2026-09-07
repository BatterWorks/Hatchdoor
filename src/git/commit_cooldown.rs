//! How long a Vault waits before its next *automatic* commit turn after one
//! failed.
//!
//! A commit turn is requested by the file watcher, so it fires as often as
//! the Vault changes. Every way a commit can fail is non-retryable. The
//! repository is invalid, or someone dirtied the enclosing checkout outside
//! the Vault's own folder, and none of those clear themselves. Without a
//! cooldown a Vault in that state would fail a turn on every save and fill
//! the log with the same error.
//!
//! The cooldown suppresses automatic requests only. A manual commit clears
//! it, and so does a commit that succeeds, so the cooldown can never outlive
//! the condition that armed it: once the operator fixes the cause, the next
//! automatic attempt goes through and clears the entry.
//!
//! Every armed window owes exactly one turn when it elapses, whether or not
//! anything changed meanwhile, and [`spawn_commit_cooldown_tick`] is what
//! issues it. That is what lets a Vault resume committing on its own: the
//! write whose commit failed is still uncommitted, so waiting for the next
//! save would leave it that way indefinitely. Further changes arriving while
//! the window is open coalesce into that same one turn.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::vault_registry::VaultId;
use crate::vault_work::{VaultWorkCoordinator, VaultWorkKind};

/// How long automatic commit turns stay suppressed after one fails.
pub const DEFAULT_COMMIT_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// How often [`spawn_commit_cooldown_tick`] looks for an elapsed window that
/// still owes its turn.
pub const COMMIT_COOLDOWN_TICK_INTERVAL: Duration = Duration::from_secs(30);

struct CooldownEntry {
    /// When automatic commit turns for this Vault are admitted again.
    until: Instant,
    /// Whether this Vault is owed a turn the moment the window elapses.
    /// [`CommitCooldown::arm`] sets it, because the write whose commit just
    /// failed is still uncommitted; suppressed requests arriving afterwards
    /// coalesce into that same one turn. Cleared only by taking the turn.
    owed: bool,
}

/// The per-Vault suppression window described in this module's own docs.
///
/// One instance for the process, shared by the watcher-forwarding path that
/// asks [`Self::try_admit`], the Vault work executor that [`Self::arm`]s and
/// [`Self::clear`]s it, and the tick that drains [`Self::due`].
pub struct CommitCooldown {
    entries: Mutex<BTreeMap<VaultId, CooldownEntry>>,
    period: Duration,
}

impl CommitCooldown {
    /// The production constructor, at [`DEFAULT_COMMIT_COOLDOWN`].
    pub fn new() -> Self {
        Self::with_period(DEFAULT_COMMIT_COOLDOWN)
    }

    /// A cooldown of an explicit length, so a test can prove the suppression
    /// and the resumption without waiting out five real minutes.
    pub fn with_period(period: Duration) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            period,
        }
    }

    /// Admit an automatic commit request for `vault_id` if its window is
    /// open, reporting whether it went through.
    ///
    /// Not a pure predicate: a refused request leaves the window owing its
    /// one turn, and an admitted one takes the turn the window owed, so
    /// [`Self::due`] never issues a second.
    pub fn try_admit(&self, vault_id: VaultId) -> bool {
        self.try_admit_at(vault_id, Instant::now())
    }

    fn try_admit_at(&self, vault_id: VaultId, now: Instant) -> bool {
        let mut entries = self.entries.lock().expect("commit cooldown poisoned");
        match entries.get_mut(&vault_id) {
            Some(entry) if entry.until > now => {
                entry.owed = true;
                false
            }
            // An elapsed entry is removed here rather than left for the tick:
            // this request is the turn it was owed.
            Some(_) => {
                entries.remove(&vault_id);
                true
            }
            None => true,
        }
    }

    /// Suppress automatic commit turns for `vault_id` for one cooldown
    /// period, after one of its commit turns failed.
    ///
    /// The window opens already owing a turn. The write that failed to commit
    /// is still uncommitted and the Vault's Git status still reads
    /// unavailable, so a Vault that is never written to again would otherwise
    /// stay that way even after the operator fixed the cause.
    pub fn arm(&self, vault_id: VaultId) {
        self.entries
            .lock()
            .expect("commit cooldown poisoned")
            .insert(
                vault_id,
                CooldownEntry {
                    until: Instant::now() + self.period,
                    owed: true,
                },
            );
    }

    /// Admit automatic commit turns for `vault_id` again, because a commit
    /// succeeded or the operator asked for one explicitly.
    pub fn clear(&self, vault_id: VaultId) {
        self.entries
            .lock()
            .expect("commit cooldown poisoned")
            .remove(&vault_id);
    }

    /// Every Vault whose window has elapsed still owing its turn, removing
    /// the entries this drains. A turn already taken by [`Self::try_admit`]
    /// leaves nothing owed, so a Vault is never asked twice for one window.
    ///
    /// A turn that fails again arms a fresh window, so a standing failure
    /// costs one attempt per cooldown period until the operator clears its
    /// cause, rather than one per save or none at all.
    pub fn due(&self, now: Instant) -> Vec<VaultId> {
        let mut entries = self.entries.lock().expect("commit cooldown poisoned");
        let elapsed = entries
            .iter()
            .filter(|(_, entry)| entry.until <= now)
            .map(|(vault_id, entry)| (*vault_id, entry.owed))
            .collect::<Vec<_>>();
        let mut due = Vec::new();
        for (vault_id, owed) in elapsed {
            entries.remove(&vault_id);
            if owed {
                due.push(vault_id);
            }
        }
        due
    }
}

impl Default for CommitCooldown {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the periodic drain that lets a Vault resume committing on its own
/// once its cooldown elapses.
///
/// Aborting the returned handle stops it; like the managed-Git scheduler's
/// own tick, this task holds no resources and issues nothing but coordinator
/// requests, which have their own shutdown draining.
pub fn spawn_commit_cooldown_tick(
    cooldown: Arc<CommitCooldown>,
    coordinator: VaultWorkCoordinator,
    tick_interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            for vault_id in cooldown.due(Instant::now()) {
                coordinator.request(vault_id, VaultWorkKind::Commit);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_id(value: &str) -> VaultId {
        value.parse().expect("valid test Vault ID")
    }

    #[test]
    fn an_unarmed_vault_is_always_admitted() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let cooldown = CommitCooldown::new();
        assert!(cooldown.try_admit(vault));
        assert!(cooldown.try_admit(vault));
    }

    #[test]
    fn an_armed_vault_suppresses_every_automatic_request_and_owes_exactly_one_turn() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let cooldown = CommitCooldown::with_period(Duration::from_secs(300));
        cooldown.arm(vault);
        for _ in 0..5 {
            assert!(
                !cooldown.try_admit(vault),
                "however many changes arrive, none is admitted during the cooldown"
            );
        }
        assert!(
            cooldown.due(Instant::now()).is_empty(),
            "the window has not elapsed yet"
        );
        assert_eq!(
            cooldown.due(Instant::now() + Duration::from_secs(301)),
            vec![vault],
            "the burst collapses into exactly one turn"
        );
        assert!(
            cooldown
                .due(Instant::now() + Duration::from_secs(600))
                .is_empty(),
            "which is owed once, not on every later tick"
        );
    }

    #[test]
    fn a_failure_with_no_further_change_is_still_retried_once_the_window_elapses() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let cooldown = CommitCooldown::with_period(Duration::from_secs(300));
        cooldown.arm(vault);
        assert_eq!(
            cooldown.due(Instant::now() + Duration::from_secs(301)),
            vec![vault],
            "the write whose commit failed is still uncommitted, so waiting \
             for a next save that may never come would strand it"
        );
        assert!(
            cooldown.try_admit(vault),
            "and the Vault is no longer suppressed"
        );
    }

    #[test]
    fn clearing_admits_the_next_request_at_once() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let cooldown = CommitCooldown::with_period(Duration::from_secs(300));
        cooldown.arm(vault);
        assert!(!cooldown.try_admit(vault));
        cooldown.clear(vault);
        assert!(
            cooldown.try_admit(vault),
            "a successful or manual commit ends the suppression immediately"
        );
        assert!(
            cooldown
                .due(Instant::now() + Duration::from_secs(600))
                .is_empty(),
            "a cleared entry owes no turn"
        );
    }

    #[test]
    fn an_elapsed_window_admits_without_waiting_for_the_tick() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let cooldown = CommitCooldown::with_period(Duration::ZERO);
        cooldown.arm(vault);
        assert!(cooldown.try_admit(vault));
        assert!(
            cooldown.due(Instant::now()).is_empty(),
            "the admitted request is the turn the entry was owed"
        );
    }

    #[test]
    fn one_vaults_cooldown_does_not_suppress_another() {
        let armed = vault_id("00000000-0000-4000-8000-000000000001");
        let healthy = vault_id("00000000-0000-4000-8000-000000000002");
        let cooldown = CommitCooldown::with_period(Duration::from_secs(300));
        cooldown.arm(armed);
        assert!(!cooldown.try_admit(armed));
        assert!(cooldown.try_admit(healthy));
    }
}

//! Egress failover for one region's game traffic.
//!
//! Accounts, sessions and keys stay on this node; an egress route only decides
//! which IP the game server sees (direct, or an HTTP CONNECT proxy on another
//! node). The pool keeps one active route and decides when to leave it.
//!
//! The decision is a differential probe, not a status-code table: a blocked IP
//! and a stale app identity can both surface as 403/426/5xx. When logins fail
//! on the active route for several accounts in a row, the same account logs in
//! once through a standby route:
//!
//! - it succeeds there: the active route is the problem, switch to the standby;
//! - it fails there too: the game or our identity is the problem, stay, and
//!   close the login gate so requests fail fast instead of hammering login.
//!
//! Standby probing is budgeted (`MAX_STANDBY_PROBES` routes, then
//! `STANDBY_PROBE_COOLDOWN`), so a real forced-upgrade window does not turn
//! into logins from every standby IP.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use reqwest::Client;
use serde::Serialize;
use tracing::{info, warn};

use crate::config::{EgressFailback, ServerRegion};
use crate::error::AppError;

/// Consecutive route-level failures (no success in between) before the active
/// route is evaluated.
const SUSPECT_FAILURES: u32 = 2;
/// Distinct accounts those failures must span, capped at the pool size, so one
/// bad account cannot indict the route on its own when others exist.
const SUSPECT_ACCOUNTS: usize = 2;
/// Standby routes probed per evaluation.
const MAX_STANDBY_PROBES: usize = 2;
/// After an evaluation that found no working standby, standbys are left alone
/// for this long.
const STANDBY_PROBE_COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// Minimum time on a route before it can be left again.
const SWITCH_DWELL: Duration = Duration::from_secs(60);
const GATE_BACKOFF_MIN: Duration = Duration::from_secs(30);
const GATE_BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
const FAILBACK_BACKOFF_MIN: Duration = Duration::from_secs(30 * 60);
const FAILBACK_BACKOFF_MAX: Duration = Duration::from_secs(6 * 60 * 60);

/// What kind of route-level failure is being reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// A login attempt failed (any error but maintenance/cookies).
    Login,
    /// A game call got no HTTP response at all.
    Call,
}

#[derive(Clone)]
pub struct EgressRoute {
    pub name: String,
    pub client: Client,
}

/// Why the login gate is closed, which decides what reopens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateCause {
    /// Logins fail on every probed route. A successful login reopens it.
    Login,
    /// Logins succeed but the game keeps rejecting the calls that follow.
    /// Only a successful game call (or a new identity) reopens it.
    Futile,
}

struct Gate {
    until: Instant,
    backoff: Duration,
    error: AppError,
    cause: GateCause,
}

#[derive(Default)]
struct State {
    /// Consecutive failed logins, and the accounts they span.
    streak: u32,
    accounts: HashSet<usize>,
    /// Consecutive game calls without any HTTP response. Kept apart from
    /// the login streak: an HTTP answer of any status proves the route
    /// delivers and clears this one, but says nothing about logins.
    call_streak: u32,
    call_accounts: HashSet<usize>,
    futile_streak: u32,
    futile_accounts: HashSet<usize>,
    gate: Option<Gate>,
    evaluating: bool,
    last_switch: Option<Instant>,
    standby_probe_after: Option<Instant>,
    probe_cursor: usize,
    failback_at: Option<Instant>,
    failback_backoff: Duration,
    hold: bool,
    last_success_ms: u64,
    last_error: Option<String>,
    last_switch_ms: u64,
    last_switch_reason: Option<String>,
    switches: u64,
}

impl State {
    fn clear_failures(&mut self) {
        self.streak = 0;
        self.accounts.clear();
        self.clear_call_failures();
    }

    fn clear_call_failures(&mut self) {
        self.call_streak = 0;
        self.call_accounts.clear();
    }

    fn clear_futility(&mut self) {
        self.futile_streak = 0;
        self.futile_accounts.clear();
    }
}

/// Snapshot for `/health` and `/internal/egress`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressStatus {
    pub active: String,
    pub primary: bool,
    pub routes: Vec<String>,
    pub hold: bool,
    pub gated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_secs_left: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub switches: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_switch_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_switch_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failback_in_secs: Option<u64>,
}

pub struct EgressPool {
    region: ServerRegion,
    routes: Vec<EgressRoute>,
    failback: EgressFailback,
    active: AtomicUsize,
    /// Bumped on every switch. Outcomes are reported with the epoch they were
    /// observed on, so a failure from the route just left is not charged to
    /// the new one.
    epoch: AtomicU64,
    /// Mirror of `State::call_streak`, so the per-response `note_route_alive`
    /// can skip the lock when there is nothing to clear.
    call_streak_hint: AtomicU32,
    state: Mutex<State>,
}

/// An in-progress route evaluation (single-flight). Dropping it without a
/// verdict — a cancelled request — just releases the slot.
pub struct Evaluation<'a> {
    pool: &'a EgressPool,
    standbys: Vec<(usize, EgressRoute)>,
}

impl Evaluation<'_> {
    pub fn standbys(&self) -> &[(usize, EgressRoute)] {
        &self.standbys
    }

    /// A standby worked where the active route did not: move to it.
    pub fn switch(self, index: usize, reason: String) {
        self.pool.apply_switch(index, reason, None);
    }

    /// No standby did better (or none was probed): the route is not the
    /// problem. Close the login gate.
    pub fn reject(self, error: &AppError) {
        let mut state = self.pool.state.lock();
        if !self.standbys.is_empty() {
            state.standby_probe_after = Some(Instant::now() + STANDBY_PROBE_COOLDOWN);
        }
        self.pool.close_gate(&mut state, error, GateCause::Login);
    }

    /// No standby reached the game either, and the trigger was not a login
    /// verdict (calls got no HTTP response): rest the standbys, leave the
    /// login gate alone.
    pub fn rest(self) {
        if !self.standbys.is_empty() {
            self.pool.state.lock().standby_probe_after =
                Some(Instant::now() + STANDBY_PROBE_COOLDOWN);
        }
    }

    /// The primary still fails: wait longer before the next failback probe.
    pub fn failback_failed(self, error: &AppError) {
        let mut state = self.pool.state.lock();
        state.failback_backoff = (state.failback_backoff * 2).min(FAILBACK_BACKOFF_MAX);
        state.failback_at = Some(Instant::now() + state.failback_backoff);
        info!(
            "{} Egress primary '{}' still failing ({}), next failback probe in {} min",
            self.pool.region_tag(),
            self.pool.routes[0].name,
            error,
            state.failback_backoff.as_secs() / 60
        );
    }
}

impl Drop for Evaluation<'_> {
    fn drop(&mut self) {
        self.pool.state.lock().evaluating = false;
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn nonzero(value: u64) -> Option<u64> {
    (value != 0).then_some(value)
}

impl EgressPool {
    /// `routes` must be non-empty; the first route is the primary.
    pub fn new(region: ServerRegion, routes: Vec<EgressRoute>, failback: EgressFailback) -> Self {
        assert!(
            !routes.is_empty(),
            "an egress pool needs at least one route"
        );
        if routes.len() > 1 {
            info!(
                "{} Egress routes: {}",
                region.as_str().to_uppercase(),
                routes
                    .iter()
                    .map(|route| route.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
        Self {
            region,
            routes,
            failback,
            active: AtomicUsize::new(0),
            epoch: AtomicU64::new(0),
            call_streak_hint: AtomicU32::new(0),
            state: Mutex::new(State::default()),
        }
    }

    /// A pool with the node's single default route (no failover).
    pub fn single(region: ServerRegion, client: Client) -> Self {
        Self::new(
            region,
            vec![EgressRoute {
                name: "default".to_string(),
                client,
            }],
            EgressFailback::Auto,
        )
    }

    fn region_tag(&self) -> String {
        self.region.as_str().to_uppercase()
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// The active route's HTTP client with the epoch it belongs to.
    pub fn active(&self) -> (u64, Client) {
        // The state lock makes (epoch, active) a consistent pair against a
        // concurrent switch.
        let _state = self.state.lock();
        let index = self.active.load(Ordering::Acquire);
        (self.epoch(), self.routes[index].client.clone())
    }

    pub fn active_name(&self) -> &str {
        &self.routes[self.active.load(Ordering::Acquire)].name
    }

    pub fn is_gated(&self) -> bool {
        self.state
            .lock()
            .gate
            .as_ref()
            .is_some_and(|gate| Instant::now() < gate.until)
    }

    /// Fail fast while the login gate is closed. When the backoff has run
    /// out, the caller that sees it becomes the probe: the gate is re-armed
    /// with a doubled backoff first, so concurrent callers stay blocked and a
    /// failing probe needs no further bookkeeping.
    pub fn check_gate(&self) -> Result<(), AppError> {
        let mut state = self.state.lock();
        let Some(gate) = state.gate.as_mut() else {
            return Ok(());
        };
        let now = Instant::now();
        if now < gate.until {
            return Err(gate.error.clone());
        }
        gate.backoff = (gate.backoff * 2).min(GATE_BACKOFF_MAX);
        gate.until = now + gate.backoff;
        Ok(())
    }

    fn close_gate(&self, state: &mut State, error: &AppError, cause: GateCause) {
        state.last_error = Some(error.to_string());
        if let Some(gate) = state.gate.as_mut() {
            // Already closed (a half-open probe failed): the backoff was
            // extended when the probe was let through. Futility outranks a
            // login verdict because a later login success must not reopen it.
            if cause == GateCause::Futile {
                gate.cause = GateCause::Futile;
            }
            gate.error = error.clone();
            return;
        }
        warn!(
            "{} Login gate closed for {} s on egress '{}': {}",
            self.region_tag(),
            GATE_BACKOFF_MIN.as_secs(),
            self.active_name(),
            error
        );
        state.gate = Some(Gate {
            until: Instant::now() + GATE_BACKOFF_MIN,
            backoff: GATE_BACKOFF_MIN,
            error: error.clone(),
            cause,
        });
    }

    /// A game call succeeded on `epoch`'s route: everything is healthy.
    pub fn note_api_success(&self, epoch: u64) {
        if epoch != self.epoch() {
            return;
        }
        let mut state = self.state.lock();
        state.last_success_ms = unix_ms();
        // Hot path: skip the resets when there is nothing to reset.
        if state.streak != 0
            || state.call_streak != 0
            || state.futile_streak != 0
            || state.gate.is_some()
        {
            self.call_streak_hint.store(0, Ordering::Relaxed);
            state.clear_failures();
            state.clear_futility();
            if state.gate.take().is_some() {
                info!("{} Login gate reopened", self.region_tag());
            }
        }
    }

    /// A login succeeded on `epoch`'s route: the route and identity are
    /// accepted at login, which says nothing yet about the calls that follow.
    pub fn note_login_success(&self, epoch: u64) {
        if epoch != self.epoch() {
            return;
        }
        let mut state = self.state.lock();
        self.call_streak_hint.store(0, Ordering::Relaxed);
        state.clear_failures();
        if state
            .gate
            .as_ref()
            .is_some_and(|gate| gate.cause == GateCause::Login)
        {
            state.gate = None;
            info!("{} Login gate reopened", self.region_tag());
        }
    }

    /// A game call on `epoch`'s route got an HTTP response (any status): the
    /// route delivers, whatever the game thinks of the request.
    pub fn note_route_alive(&self, epoch: u64) {
        if self.call_streak_hint.load(Ordering::Relaxed) == 0 || epoch != self.epoch() {
            return;
        }
        self.call_streak_hint.store(0, Ordering::Relaxed);
        self.state.lock().clear_call_failures();
    }

    /// Record a route-level failure for `account` on `epoch`'s route. Returns
    /// an evaluation when the failures now indict the route and no evaluation
    /// is running; the caller probes its standbys and reports the verdict.
    pub fn note_failure(
        &self,
        epoch: u64,
        kind: FailureKind,
        account: usize,
        error: &AppError,
        pool_len: usize,
    ) -> Option<Evaluation<'_>> {
        if epoch != self.epoch() {
            return None;
        }
        let mut state = self.state.lock();
        state.last_error = Some(error.to_string());
        let normal = (SUSPECT_FAILURES, SUSPECT_ACCOUNTS.min(pool_len.max(1)));
        let (streak, accounts, (failures_needed, accounts_needed)) = match kind {
            FailureKind::Login => {
                state.streak += 1;
                state.accounts.insert(account);
                // Behind a closed gate every login is already a rationed
                // probe, and its failure is news (logins may have worked when
                // the gate closed), so one is enough to look at the standbys.
                let needed = if state.gate.is_some() { (1, 1) } else { normal };
                (state.streak, state.accounts.len(), needed)
            }
            FailureKind::Call => {
                state.call_streak += 1;
                state.call_accounts.insert(account);
                self.call_streak_hint
                    .store(state.call_streak, Ordering::Relaxed);
                (state.call_streak, state.call_accounts.len(), normal)
            }
        };
        if streak < failures_needed || accounts < accounts_needed || state.evaluating {
            return None;
        }
        state.evaluating = true;
        let now = Instant::now();
        let dwelled = state
            .last_switch
            .is_none_or(|at| now.duration_since(at) >= SWITCH_DWELL);
        let rested = state.standby_probe_after.is_none_or(|at| now >= at);
        let mut standbys = Vec::new();
        if dwelled && rested && self.routes.len() > 1 {
            let active = self.active.load(Ordering::Acquire);
            let count = self.routes.len();
            // Rotate so successive evaluations cover different standbys.
            let start = state.probe_cursor;
            for offset in 0..count {
                let index = (start + offset) % count;
                if index == active {
                    continue;
                }
                standbys.push((index, self.routes[index].clone()));
                if standbys.len() == MAX_STANDBY_PROBES {
                    state.probe_cursor = (index + 1) % count;
                    break;
                }
            }
        }
        Some(Evaluation {
            pool: self,
            standbys,
        })
    }

    /// A recovery (relogin) succeeded but the game rejected the retried call
    /// with the same error. The route passes login, so standbys are not
    /// probed; after enough of these the gate closes until a game call works.
    pub fn note_futile(&self, epoch: u64, account: usize, error: &AppError, pool_len: usize) {
        if epoch != self.epoch() {
            return;
        }
        let mut state = self.state.lock();
        state.futile_streak += 1;
        state.futile_accounts.insert(account);
        let accounts_needed = SUSPECT_ACCOUNTS.min(pool_len.max(1));
        if state.futile_streak >= SUSPECT_FAILURES && state.futile_accounts.len() >= accounts_needed
        {
            self.close_gate(&mut state, error, GateCause::Futile);
        }
    }

    /// Forget the current verdict: the identity or the account set changed,
    /// so the next login deserves a fresh attempt.
    pub fn reset_gate(&self) {
        let mut state = self.state.lock();
        state.clear_failures();
        state.clear_futility();
        if state.gate.take().is_some() {
            info!("{} Login gate reset", self.region_tag());
        }
    }

    fn apply_switch(&self, index: usize, reason: String, hold: Option<bool>) {
        let mut state = self.state.lock();
        let previous = self.active.swap(index, Ordering::AcqRel);
        if let Some(hold) = hold {
            state.hold = hold;
        }
        state.clear_failures();
        state.clear_futility();
        state.gate = None;
        if previous == index {
            return;
        }
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.call_streak_hint.store(0, Ordering::Relaxed);
        state.last_switch = Some(Instant::now());
        state.last_switch_ms = unix_ms();
        state.last_switch_reason = Some(reason.clone());
        state.switches += 1;
        if index == 0 {
            state.failback_at = None;
        } else {
            state.failback_backoff = FAILBACK_BACKOFF_MIN;
            state.failback_at = Some(Instant::now() + FAILBACK_BACKOFF_MIN);
        }
        warn!(
            "{} Egress switched '{}' -> '{}': {}",
            self.region_tag(),
            self.routes[previous].name,
            self.routes[index].name,
            reason
        );
    }

    /// Operator switch. `hold` keeps automatic failback off until cleared;
    /// failover away from a held route still happens.
    pub fn switch_to(&self, name: &str, hold: Option<bool>) -> Result<bool, AppError> {
        let index = self
            .routes
            .iter()
            .position(|route| route.name == name)
            .ok_or_else(|| AppError::NotFound(format!("egress '{name}'")))?;
        let changed = self.active.load(Ordering::Acquire) != index;
        self.apply_switch(index, "operator request".to_string(), hold);
        Ok(changed)
    }

    pub fn set_hold(&self, hold: bool) {
        self.state.lock().hold = hold;
    }

    /// The primary route, when it is time to see whether it works again.
    pub fn failback_due(&self) -> Option<(EgressRoute, Evaluation<'_>)> {
        if self.failback != EgressFailback::Auto || self.active.load(Ordering::Acquire) == 0 {
            return None;
        }
        let mut state = self.state.lock();
        if state.hold || state.evaluating || state.failback_at.is_none_or(|at| Instant::now() < at)
        {
            return None;
        }
        state.evaluating = true;
        Some((
            self.routes[0].clone(),
            Evaluation {
                pool: self,
                standbys: Vec::new(),
            },
        ))
    }

    pub fn status(&self) -> EgressStatus {
        let state = self.state.lock();
        let active = self.active.load(Ordering::Acquire);
        let now = Instant::now();
        // A gate whose backoff has run out no longer refuses anything: the
        // next login is let through as the probe.
        let gate_left = state
            .gate
            .as_ref()
            .filter(|gate| now < gate.until)
            .map(|gate| gate.until.saturating_duration_since(now).as_secs());
        EgressStatus {
            active: self.routes[active].name.clone(),
            primary: active == 0,
            routes: self.routes.iter().map(|route| route.name.clone()).collect(),
            hold: state.hold,
            gated: gate_left.is_some(),
            gate_secs_left: gate_left,
            last_success_at: nonzero(state.last_success_ms),
            last_error: state.last_error.clone(),
            switches: state.switches,
            last_switch_at: nonzero(state.last_switch_ms),
            last_switch_reason: state.last_switch_reason.clone(),
            failback_in_secs: state
                .failback_at
                .filter(|_| active != 0 && !state.hold && self.failback == EgressFailback::Auto)
                .map(|at| at.saturating_duration_since(now).as_secs()),
        }
    }

    #[cfg(test)]
    pub(crate) fn expire_timers_for_test(&self) {
        let mut state = self.state.lock();
        let past = Instant::now() - Duration::from_secs(1);
        if let Some(gate) = state.gate.as_mut() {
            gate.until = past;
        }
        if state.failback_at.is_some() {
            state.failback_at = Some(past);
        }
        state.last_switch = None;
        state.standby_probe_after = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(routes: usize, failback: EgressFailback) -> EgressPool {
        let routes = (0..routes)
            .map(|index| EgressRoute {
                name: format!("r{index}"),
                client: Client::new(),
            })
            .collect();
        EgressPool::new(ServerRegion::Cn, routes, failback)
    }

    fn session_error() -> AppError {
        AppError::SessionError
    }

    fn standby_names(evaluation: &Evaluation<'_>) -> Vec<String> {
        evaluation
            .standbys()
            .iter()
            .map(|(_, route)| route.name.clone())
            .collect()
    }

    // One bad account must not indict the route while the pool has others;
    // a second account failing in a row does.
    #[test]
    fn evaluation_needs_consecutive_failures_across_accounts() {
        let pool = pool(3, EgressFailback::Auto);
        let epoch = pool.epoch();
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 5)
            .is_none());
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 5)
            .is_none());
        pool.note_api_success(epoch);
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 5)
            .is_none());
        let evaluation = pool
            .note_failure(epoch, FailureKind::Login, 2, &session_error(), 5)
            .expect("second account failing in a row triggers evaluation");
        assert_eq!(standby_names(&evaluation), ["r1", "r2"]);
    }

    #[test]
    fn single_account_pool_evaluates_after_two_failures() {
        let pool = pool(2, EgressFailback::Auto);
        let epoch = pool.epoch();
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 1)
            .is_none());
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 1)
            .is_some());
    }

    // Evaluations are single-flight, and a dropped (cancelled) one frees the slot.
    #[test]
    fn evaluation_is_single_flight_and_released_on_drop() {
        let pool = pool(2, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 2);
        let first = pool.note_failure(epoch, FailureKind::Login, 2, &session_error(), 2);
        assert!(first.is_some());
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 3, &session_error(), 2)
            .is_none());
        drop(first);
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 3, &session_error(), 2)
            .is_some());
    }

    #[test]
    fn switch_moves_route_bumps_epoch_and_ignores_stale_outcomes() {
        let pool = pool(2, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 2);
        let evaluation = pool
            .note_failure(epoch, FailureKind::Login, 2, &session_error(), 2)
            .unwrap();
        evaluation.switch(1, "test".to_string());
        assert_eq!(pool.active_name(), "r1");
        assert_eq!(pool.epoch(), epoch + 1);
        // Failures observed on the route just left are not charged to r1.
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 2)
            .is_none());
        assert!(pool
            .note_failure(epoch, FailureKind::Login, 2, &session_error(), 2)
            .is_none());
        let status = pool.status();
        assert_eq!(status.active, "r1");
        assert!(!status.primary);
        assert_eq!(status.switches, 1);
        assert!(status.failback_in_secs.is_some());
        assert!(!status.gated);
    }

    // A verdict that the route is not at fault closes the gate, rests the
    // standbys, and lets exactly one probe through per backoff.
    #[test]
    fn reject_closes_gate_and_half_opens_with_backoff() {
        let pool = pool(2, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &AppError::UpgradeRequired, 2);
        let evaluation = pool
            .note_failure(epoch, FailureKind::Login, 2, &AppError::UpgradeRequired, 2)
            .unwrap();
        assert_eq!(evaluation.standbys().len(), 1);
        evaluation.reject(&AppError::UpgradeRequired);
        assert!(pool.is_gated());
        assert!(matches!(pool.check_gate(), Err(AppError::UpgradeRequired)));

        // Standbys are resting: the next evaluation probes none.
        let again = pool
            .note_failure(epoch, FailureKind::Login, 1, &AppError::UpgradeRequired, 2)
            .unwrap();
        assert!(again.standbys().is_empty());
        again.reject(&AppError::UpgradeRequired);

        pool.state.lock().gate.as_mut().unwrap().until = Instant::now() - Duration::from_secs(1);
        assert!(pool.check_gate().is_ok(), "expired gate lets one probe in");
        assert!(pool.check_gate().is_err(), "and re-arms for everyone else");
        assert_eq!(
            pool.state.lock().gate.as_ref().unwrap().backoff,
            GATE_BACKOFF_MIN * 2
        );

        pool.note_login_success(epoch);
        assert!(!pool.is_gated());
        assert!(pool.check_gate().is_ok());
    }

    // Logins that succeed without making the game accept calls must not
    // reopen the gate; only a working game call or a reset does.
    #[test]
    fn futile_recoveries_gate_until_a_call_succeeds() {
        let pool = pool(2, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_futile(epoch, 1, &AppError::UpgradeRequired, 2);
        assert!(!pool.is_gated());
        pool.note_futile(epoch, 2, &AppError::UpgradeRequired, 2);
        assert!(pool.is_gated());
        pool.note_login_success(epoch);
        assert!(pool.is_gated(), "login success does not clear futility");
        pool.note_api_success(epoch);
        assert!(!pool.is_gated());

        pool.note_futile(epoch, 1, &AppError::UpgradeRequired, 2);
        pool.note_futile(epoch, 2, &AppError::UpgradeRequired, 2);
        assert!(pool.is_gated());
        pool.reset_gate();
        assert!(!pool.is_gated());
    }

    // The incident shape: calls are rejected while logins still work (gate
    // closed for futility), then logins start failing too. The first failed
    // probe behind the gate must reach the standbys, not wait for a streak.
    #[test]
    fn a_failed_probe_behind_the_gate_evaluates_at_once() {
        let pool = pool(2, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_futile(epoch, 1, &AppError::UpgradeRequired, 5);
        pool.note_futile(epoch, 2, &AppError::UpgradeRequired, 5);
        assert!(pool.is_gated());
        let evaluation = pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 5)
            .expect("one failure is enough behind the gate");
        assert_eq!(standby_names(&evaluation), ["r1"]);
        evaluation.switch(1, "test".to_string());
        assert!(!pool.is_gated(), "a switch starts with a clean slate");
    }

    #[test]
    fn dwell_keeps_a_fresh_route_from_being_left() {
        let pool = pool(3, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 2);
        pool.note_failure(epoch, FailureKind::Login, 2, &session_error(), 2)
            .unwrap()
            .switch(1, "test".to_string());
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 2);
        let evaluation = pool
            .note_failure(epoch, FailureKind::Login, 2, &session_error(), 2)
            .unwrap();
        assert!(evaluation.standbys().is_empty(), "inside the dwell window");
        drop(evaluation);
        pool.expire_timers_for_test();
        let evaluation = pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 2)
            .unwrap();
        assert_eq!(standby_names(&evaluation), ["r0", "r2"]);
    }

    // Call failures (no HTTP response) have their own streak: any HTTP answer
    // clears it, a rejected call included, and they never use the shortcut
    // behind a gate. Otherwise sporadic timeouts during a 426 window would
    // add up and move the region to a standby that cannot help.
    #[test]
    fn call_failures_need_a_streak_no_http_answer_interrupts() {
        let pool = pool(2, EgressFailback::Auto);
        let epoch = pool.epoch();
        let timeout = AppError::NetworkError("timeout".to_string());
        assert!(pool
            .note_failure(epoch, FailureKind::Call, 1, &timeout, 5)
            .is_none());
        pool.note_route_alive(epoch);
        assert!(pool
            .note_failure(epoch, FailureKind::Call, 2, &timeout, 5)
            .is_none());

        // Behind a futility gate one call failure is still not enough.
        pool.note_futile(epoch, 1, &AppError::UpgradeRequired, 5);
        pool.note_futile(epoch, 2, &AppError::UpgradeRequired, 5);
        assert!(pool.is_gated());
        pool.note_route_alive(epoch);
        assert!(pool
            .note_failure(epoch, FailureKind::Call, 1, &timeout, 5)
            .is_none());
        assert!(pool.is_gated(), "an HTTP answer does not reopen the gate");

        // Two in a row across accounts do indict the route, and the login
        // streak is untouched by them.
        let evaluation = pool
            .note_failure(epoch, FailureKind::Call, 2, &timeout, 5)
            .expect("route is not delivering");
        assert_eq!(standby_names(&evaluation), ["r1"]);
        drop(evaluation);
        assert_eq!(pool.state.lock().streak, 0);
    }

    // With four or more routes and a non-primary active route, every standby
    // must come up eventually (the cursor once got stuck on one pair).
    #[test]
    fn rotation_reaches_every_standby_from_a_non_primary_route() {
        let pool = pool(4, EgressFailback::Auto);
        pool.switch_to("r2", None).unwrap();
        pool.expire_timers_for_test();
        let epoch = pool.epoch();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..3 {
            pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 2);
            let evaluation = pool
                .note_failure(epoch, FailureKind::Login, 2, &session_error(), 2)
                .unwrap();
            seen.extend(standby_names(&evaluation));
            evaluation.reject(&session_error());
            pool.expire_timers_for_test();
            pool.reset_gate();
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            ["r0", "r1", "r3"],
            "no standby is skipped forever"
        );
    }

    // /health must not call a region gated once the backoff has run out: with
    // no traffic nothing would ever clear it.
    #[test]
    fn an_expired_gate_is_not_reported_as_gated() {
        let pool = pool(1, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 1);
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 1)
            .unwrap()
            .reject(&session_error());
        let status = pool.status();
        assert!(status.gated && status.gate_secs_left.is_some());
        pool.expire_timers_for_test();
        let status = pool.status();
        assert!(!status.gated && status.gate_secs_left.is_none());
    }

    #[test]
    fn standby_probes_rotate_across_evaluations() {
        let pool = pool(5, EgressFailback::Auto);
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 2);
        let first = pool
            .note_failure(epoch, FailureKind::Login, 2, &session_error(), 2)
            .unwrap();
        assert_eq!(standby_names(&first), ["r1", "r2"]);
        first.reject(&session_error());
        pool.expire_timers_for_test();
        let second = pool
            .note_failure(epoch, FailureKind::Login, 1, &session_error(), 2)
            .unwrap();
        assert_eq!(standby_names(&second), ["r3", "r4"]);
    }

    #[test]
    fn failback_probes_primary_on_schedule_and_backs_off() {
        let pool = pool(2, EgressFailback::Auto);
        assert!(pool.failback_due().is_none(), "already on the primary");
        pool.switch_to("r1", None).unwrap();
        assert!(pool.failback_due().is_none(), "not due yet");
        pool.expire_timers_for_test();
        let (primary, evaluation) = pool.failback_due().expect("due");
        assert_eq!(primary.name, "r0");
        assert!(pool.failback_due().is_none(), "single-flight");
        evaluation.failback_failed(&session_error());
        assert_eq!(pool.state.lock().failback_backoff, FAILBACK_BACKOFF_MIN * 2);
        assert!(pool.failback_due().is_none());

        pool.expire_timers_for_test();
        let (_, evaluation) = pool.failback_due().unwrap();
        evaluation.switch(0, "failback".to_string());
        assert_eq!(pool.active_name(), "r0");
        assert!(pool.status().failback_in_secs.is_none());
    }

    #[test]
    fn hold_and_manual_mode_suppress_failback() {
        let held = pool(2, EgressFailback::Auto);
        assert!(held.switch_to("r1", Some(true)).unwrap());
        held.expire_timers_for_test();
        assert!(held.failback_due().is_none());
        assert!(held.status().hold);
        held.set_hold(false);
        assert!(held.failback_due().is_some());

        let manual = pool(2, EgressFailback::Manual);
        manual.switch_to("r1", None).unwrap();
        manual.expire_timers_for_test();
        assert!(manual.failback_due().is_none());
        assert!(matches!(
            manual.switch_to("nope", None),
            Err(AppError::NotFound(_))
        ));
        assert!(!manual.switch_to("r1", None).unwrap(), "already active");
    }

    #[test]
    fn single_route_pool_gates_without_probing() {
        let pool = EgressPool::single(ServerRegion::Jp, Client::new());
        assert_eq!(pool.len(), 1);
        assert!(!pool.is_empty());
        let epoch = pool.epoch();
        pool.note_failure(epoch, FailureKind::Login, 1, &session_error(), 2);
        let evaluation = pool
            .note_failure(epoch, FailureKind::Login, 2, &session_error(), 2)
            .unwrap();
        assert!(evaluation.standbys().is_empty());
        evaluation.reject(&session_error());
        let status = pool.status();
        assert!(status.gated && status.primary);
        assert_eq!(status.routes, ["default"]);
        assert!(status.last_error.is_some());
    }
}

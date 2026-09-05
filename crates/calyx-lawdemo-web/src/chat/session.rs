//! The EPISODE REGISTRY — the fix for #2061.
//!
//! # What was wrong
//!
//! An episode used to exist only as a `tokio::spawn`ed task writing into the mpsc
//! channel behind ONE `POST /api/chat` response body. That made the HTTP response the
//! episode's only channel, in the worst possible direction:
//!
//!   * the episode could not observe its client (`ChannelSink` dropped send errors), so
//!     a closed tab, a discarded tab, or a dropped tunnel left it running to the full
//!     wall budget — burning DeepSeek tokens and holding the process-wide GPU search
//!     permit with nobody listening (measured: alive 2 minutes after `kill -9`);
//!   * and the client could not observe its episode after the stream broke, because
//!     there was no way to ask for it again. Chrome FREEZES a hidden tab, and a frozen
//!     page's "freezable tasks ... like JavaScript timers and fetch callbacks don't
//!     run"; it may then DISCARD the page entirely. Mobile browsers suspend the
//!     renderer and drop the sockets. Any of those permanently lost the research.
//!
//! # The shape now
//!
//! The episode is a first-class server-side object and an HTTP response is merely an
//! ATTACHMENT to it:
//!
//!   * every event gets a monotonic `seq` and is stored, pre-serialized, in a bounded
//!     replay log, and simultaneously broadcast to whoever is attached;
//!   * `POST /api/chat` opens an episode and attaches at seq 0; `GET /api/chat/stream`
//!     attaches an existing episode at any `from_seq`. Both use ONE stream
//!     implementation, so a resume can never drift from a first connection;
//!   * attachments are COUNTED. An episode with no attachment for `client_grace` is
//!     cancelled as abandoned — that is the bound on orphaned spend;
//!   * a finished episode is RETAINED for `episode_retention`, so a client that was
//!     frozen across the finish line still collects the answer it paid for.
//!
//! Single-process, single-replica, loopback service: the log lives in memory, and that
//! is the honest design here — no database, no per-token write amplification, and no
//! cross-replica routing problem to solve, because there is exactly one replica.
//!
//! Nothing here is best-effort. A resume that cannot be served completely is REFUSED
//! with `CALYX_LAWDEMO_EPISODE_REPLAY_TRUNCATED`; an unknown id is refused with
//! `CALYX_LAWDEMO_EPISODE_UNKNOWN`. A short stream is never passed off as a whole one.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rand::Rng;
use tokio::sync::{broadcast, oneshot};

use crate::chat::event::{ChatEvent, EventSink};
use crate::error::{ApiError, ErrorCode};

/// Live-tail fan-out depth. An attached client that falls this far behind is not
/// silently skipped: `recv` reports `Lagged`, the stream fails closed, and the client
/// reattaches from its own `last_seq` — which is served from the replay log, correctly.
const LIVE_CHANNEL_CAPACITY: usize = 1024;

/// How often the reaper wakes. One second keeps abandonment detection prompt without
/// being a meaningful cost on an otherwise idle box.
pub const REAP_INTERVAL: Duration = Duration::from_secs(1);

/// One episode event, sequenced and serialized ONCE at publish time. Serializing here
/// (not per attachment) means a replay costs a clone of an `Arc`, and the replay-log
/// byte accounting is exact rather than estimated.
#[derive(Debug)]
pub struct Frame {
    /// Monotonic per-episode sequence, starting at 1. Emitted as the SSE `id:` field
    /// and echoed back by a resuming client as `from_seq`/`Last-Event-ID`.
    pub seq: u64,
    /// The SSE `event:` name.
    pub name: &'static str,
    /// The serialized SSE `data:` JSON payload.
    pub data: String,
    /// Whether this frame ends the episode (`done`). The attachment stream closes on
    /// it, so a client never waits on a stream that has nothing left to say.
    pub terminal: bool,
}

impl Frame {
    /// Render as an axum SSE event, carrying `id:` so the browser's own `Last-Event-ID`
    /// and our explicit `from_seq` agree on the resume point.
    pub fn to_sse(&self) -> axum::response::sse::Event {
        axum::response::sse::Event::default()
            .id(self.seq.to_string())
            .event(self.name)
            .data(&self.data)
    }
}

/// Why an episode was cancelled. Both are user-visible outcomes, and they read very
/// differently in the UI, so they are distinct rather than one boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// The user pressed Stop, started a new chat, or closed the tab.
    UserRequest,
    /// No client has been attached for longer than the configured grace period.
    ClientGone,
}

impl CancelReason {
    /// The static, secret-free note rendered in the terminal `cancelled` event.
    pub const fn note(self) -> &'static str {
        match self {
            Self::UserRequest => {
                "Episode stopped at your request. No authority was written; any in-flight \
                 Calyx tool subprocess was terminated."
            }
            Self::ClientGone => {
                "Episode stopped because this browser was disconnected for longer than the \
                 configured grace period. No authority was written; any in-flight Calyx tool \
                 subprocess was terminated. Ask the question again to restart it."
            }
        }
    }

    /// Stable wire token for logs and the cancel acknowledgement.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserRequest => "user_request",
            Self::ClientGone => "client_gone",
        }
    }
}

/// A single running-or-retained episode.
struct Episode {
    /// Sequenced, pre-serialized events, oldest first. Trimmed from the FRONT when it
    /// would exceed the byte cap, so the live tail and the terminal event always
    /// survive — the part a reconnecting client most needs.
    log: Vec<Arc<Frame>>,
    /// Current byte weight of `log` (the `data` payloads).
    log_bytes: usize,
    /// The lowest `seq` still present in `log`. A resume from below this is REFUSED,
    /// never quietly served with a hole.
    replay_floor: u64,
    /// Next sequence to assign.
    next_seq: u64,
    /// Live fan-out to attached clients.
    live: broadcast::Sender<Arc<Frame>>,
    /// One-shot cancel into the episode task. Taken when fired or when the episode
    /// ends naturally, so a cancel after the fact is a truthful "already finished".
    cancel: Option<oneshot::Sender<CancelReason>>,
    /// Number of currently attached HTTP responses.
    attached: usize,
    /// When `attached` last fell to zero (`None` while someone is attached). The
    /// reaper's clock for the abandoned-episode rule.
    idle_since: Option<Instant>,
    /// Set when the terminal `done` frame is published.
    finished_at: Option<Instant>,
    /// When the episode was opened — the leak detector's clock.
    opened_at: Instant,
    /// Byte cap for `log`, captured at open time from the validated config.
    replay_max_bytes: usize,
}

impl Episode {
    fn is_terminal(&self) -> bool {
        self.finished_at.is_some()
    }
}

/// The process-wide registry. In-memory and ephemeral by design: episode ids never
/// leave the process, nothing is persisted, and a restart correctly loses everything
/// (the episodes died with it).
static REGISTRY: LazyLock<Mutex<HashMap<String, Episode>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn registry() -> MutexGuard<'static, HashMap<String, Episode>> {
    // A poisoned registry means a panic happened while holding it, which would leave
    // sequencing in an unknown state. There is no safe way to continue, and pretending
    // otherwise is exactly the "faked green" this repo refuses.
    REGISTRY.lock().expect("episode registry lock is poisoned")
}

/// A fresh, unguessable episode id.
///
/// This id IS the capability to attach to a running episode's event stream. The old
/// `ep-{counter}` scheme was fine when the id only addressed a cancel that the same
/// client had just been handed; it is not fine now that the id can be used to READ an
/// episode. 128 bits of randomness from the OS-seeded generator makes enumeration by
/// another visitor of this unauthenticated surface a non-starter.
fn new_episode_id() -> String {
    format!("ep-{:032x}", rand::rng().random::<u128>())
}

/// Open a new episode. Returns its id and the cancel receiver for the episode task.
pub fn open(replay_max_bytes: usize) -> (String, oneshot::Receiver<CancelReason>) {
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let (live, _) = broadcast::channel(LIVE_CHANNEL_CAPACITY);
    let id = new_episode_id();
    let now = Instant::now();
    registry().insert(
        id.clone(),
        Episode {
            log: Vec::new(),
            log_bytes: 0,
            replay_floor: 1,
            next_seq: 1,
            live,
            cancel: Some(cancel_tx),
            attached: 0,
            // An episode is born unattached: if the client that opened it never manages
            // to read the response, the reaper must still be able to collect it.
            idle_since: Some(now),
            finished_at: None,
            opened_at: now,
            replay_max_bytes,
        },
    );
    tracing::info!(episode = %id, "episode opened");
    (id, cancel_rx)
}

/// Sequence, store, and broadcast one event.
///
/// A publish to an id the registry no longer holds is a REAL invariant break (the task
/// outlived its own registration), so it is logged at error level rather than dropped
/// quietly.
pub fn publish(episode_id: &str, event: ChatEvent) {
    let mut reg = registry();
    let Some(ep) = reg.get_mut(episode_id) else {
        tracing::error!(
            episode = %episode_id,
            event = event.name(),
            "CALYX_LAWDEMO_INTERNAL: publish to an episode the registry no longer holds"
        );
        return;
    };
    let seq = ep.next_seq;
    ep.next_seq += 1;
    let terminal = matches!(event, ChatEvent::Done);
    let frame = Arc::new(Frame {
        seq,
        name: event.name(),
        data: serde_json::to_string(&event.data()).unwrap_or_else(|error| {
            // A ChatEvent payload that will not serialize is a code defect, not a
            // runtime condition. Say so loudly and keep the sequence intact.
            tracing::error!(
                episode = %episode_id,
                event = event.name(),
                %error,
                "CALYX_LAWDEMO_INTERNAL: chat event payload failed to serialize"
            );
            "{}".to_owned()
        }),
        terminal,
    });

    ep.log_bytes += frame.data.len();
    ep.log.push(Arc::clone(&frame));
    // Trim from the front so the live tail and the terminal event always survive. Every
    // dropped frame raises `replay_floor`, which is what makes a resume from inside the
    // lost range a hard refusal instead of a hole.
    while ep.log_bytes > ep.replay_max_bytes && ep.log.len() > 1 {
        let dropped = ep.log.remove(0);
        ep.log_bytes -= dropped.data.len();
        ep.replay_floor = dropped.seq + 1;
        tracing::warn!(
            episode = %episode_id,
            dropped_seq = dropped.seq,
            replay_floor = ep.replay_floor,
            "episode replay log exceeded its byte cap; a resume from before the floor will be refused"
        );
    }

    if terminal {
        ep.finished_at = Some(Instant::now());
        // The episode is over; nothing can cancel it any more.
        ep.cancel = None;
        tracing::info!(
            episode = %episode_id,
            events = seq,
            log_bytes = ep.log_bytes,
            "episode finished; retained for reattach"
        );
    }

    // A send error here only means nobody is attached right now. That is normal and
    // recoverable BECAUSE the frame is already in the replay log above.
    let _ = ep.live.send(frame);
}

/// Guarantee the episode reaches a terminal frame.
///
/// Every exit from the agent loop emits its own `done` today, so this is normally a
/// no-op. It exists so that the property clients depend on — *an attached stream always
/// ends with a terminal event, it never just stops* — is enforced at the transport
/// boundary and cannot be broken by a future edit to the agent loop.
pub fn finish_if_running(episode_id: &str) {
    let running = {
        let reg = registry();
        match reg.get(episode_id) {
            Some(ep) => !ep.is_terminal(),
            None => false,
        }
    };
    if running {
        tracing::warn!(
            episode = %episode_id,
            "episode task ended without a terminal event; publishing `done` at the transport boundary"
        );
        publish(episode_id, ChatEvent::Done);
    }
}

/// Everything an attached HTTP response needs to serve a complete, gap-free stream.
pub struct Attachment {
    /// Frames already published, from the requested resume point onward.
    pub replay: Vec<Arc<Frame>>,
    /// The live tail, subscribed BEFORE `replay` was snapshotted under the same lock,
    /// so no frame can slip between the two and none can be delivered twice.
    pub live: broadcast::Receiver<Arc<Frame>>,
    /// Decrements the attachment count on drop — including when axum drops the
    /// response body because the client disconnected.
    pub guard: AttachGuard,
}

/// Attachment accounting. Dropping this is what tells the reaper a client is gone.
pub struct AttachGuard {
    episode_id: String,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        let mut reg = registry();
        let Some(ep) = reg.get_mut(&self.episode_id) else {
            return;
        };
        ep.attached = ep.attached.saturating_sub(1);
        if ep.attached == 0 {
            ep.idle_since = Some(Instant::now());
            tracing::info!(
                episode = %self.episode_id,
                terminal = ep.is_terminal(),
                "episode detached; abandonment clock started"
            );
        }
    }
}

/// Attach to an episode at `from_seq` (0 = from the beginning).
///
/// Fails closed on an unknown id and on a resume point the log can no longer cover.
pub fn attach(episode_id: &str, from_seq: u64) -> Result<Attachment, ApiError> {
    let mut reg = registry();
    let Some(ep) = reg.get_mut(episode_id) else {
        return Err(ApiError::of(ErrorCode::EpisodeUnknown));
    };
    // `from_seq` is the last seq the client ALREADY has, so it wants seq > from_seq.
    // That is servable iff the log still holds `from_seq + 1` — i.e. the floor has not
    // risen past it. An episode that has published nothing yet trivially qualifies.
    let wanted_first = from_seq + 1;
    if wanted_first < ep.replay_floor {
        return Err(
            ApiError::of(ErrorCode::EpisodeReplayTruncated).with_upstream(serde_json::json!({
                "episode_id": episode_id,
                "requested_from_seq": from_seq,
                "earliest_available_seq": ep.replay_floor,
            })),
        );
    }
    // Subscribe first, snapshot second, both under this one lock: `publish` needs the
    // same lock, so no frame can be published between the two steps — the replay and
    // the live tail meet exactly, with no gap and no overlap.
    let live = ep.live.subscribe();
    let replay: Vec<Arc<Frame>> = ep
        .log
        .iter()
        .filter(|f| f.seq >= wanted_first)
        .cloned()
        .collect();

    ep.attached += 1;
    ep.idle_since = None;
    tracing::info!(
        episode = %episode_id,
        from_seq,
        replay_frames = replay.len(),
        attached = ep.attached,
        "episode attached"
    );
    Ok(Attachment {
        replay,
        live,
        guard: AttachGuard {
            episode_id: episode_id.to_owned(),
        },
    })
}

/// The outcome of a cancel request, so the caller can tell the three cases apart
/// instead of collapsing them into one boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The signal was delivered; a terminal `cancelled` + `done` will follow.
    Signalled,
    /// The episode exists but has already reached its terminal event.
    AlreadyFinished,
    /// No episode with that id is held (never existed, or aged out).
    Unknown,
}

impl CancelOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Signalled => "signalled",
            Self::AlreadyFinished => "already_finished",
            Self::Unknown => "unknown",
        }
    }
}

/// Fire the cancel signal for `episode_id`.
pub fn cancel(episode_id: &str, reason: CancelReason) -> CancelOutcome {
    let mut reg = registry();
    let Some(ep) = reg.get_mut(episode_id) else {
        return CancelOutcome::Unknown;
    };
    if ep.is_terminal() {
        return CancelOutcome::AlreadyFinished;
    }
    // A closed receiver means the episode task is already unwinding, so the signal has
    // nowhere to land and the honest answer is "already finished".
    match ep.cancel.take().map(|tx| tx.send(reason)) {
        Some(Ok(())) => {
            tracing::info!(episode = %episode_id, reason = reason.as_str(), "episode cancel signalled");
            CancelOutcome::Signalled
        }
        _ => CancelOutcome::AlreadyFinished,
    }
}

/// One reaper pass. Returns `(cancelled_abandoned, evicted)` for logging and FSV.
///
/// Two rules, both of which exist because a browser cannot be trusted to say goodbye:
///   * a RUNNING episode unattached for longer than `grace` is cancelled. This is the
///     bound on orphaned DeepSeek spend and orphaned GPU search permits.
///   * a FINISHED episode is evicted `retention` after it finished, freeing its log.
///
/// Note that a merely FROZEN tab still holds its TCP connection, so it still counts as
/// attached and is never reaped — only a genuinely gone client (closed tab, discarded
/// page, dropped tunnel) trips the first rule.
pub fn reap(grace: Duration, retention: Duration, leak_after: Duration) -> (usize, usize) {
    let now = Instant::now();
    let mut to_cancel: Vec<String> = Vec::new();
    let mut evicted = 0usize;

    {
        let mut reg = registry();
        reg.retain(|id, ep| {
            if let Some(finished) = ep.finished_at {
                if now.duration_since(finished) >= retention {
                    tracing::info!(episode = %id, "retained episode evicted after its retention window");
                    evicted += 1;
                    return false;
                }
                return true;
            }
            // A running episode that outlived every budget it has never terminated. It
            // cannot be cancelled (the task is gone or wedged), so evicting it is the
            // only way to free the log — and it is a genuine defect, said loudly.
            if now.duration_since(ep.opened_at) >= leak_after {
                tracing::error!(
                    episode = %id,
                    age_secs = now.duration_since(ep.opened_at).as_secs(),
                    attached = ep.attached,
                    "CALYX_LAWDEMO_INTERNAL: a running episode outlived its wall budget without \
                     publishing a terminal event; evicting the leaked registration"
                );
                evicted += 1;
                return false;
            }
            if ep.attached == 0
                && let Some(idle) = ep.idle_since
                && now.duration_since(idle) >= grace
            {
                to_cancel.push(id.clone());
            }
            true
        });
    }

    let mut cancelled = 0usize;
    for id in to_cancel {
        if cancel(&id, CancelReason::ClientGone) == CancelOutcome::Signalled {
            tracing::warn!(
                episode = %id,
                grace_secs = grace.as_secs(),
                "episode cancelled: no client attached for the full grace period"
            );
            cancelled += 1;
        }
    }
    (cancelled, evicted)
}

/// Non-secret registry counts for `/api/health`, so abandonment and retention are
/// observable in production rather than inferred.
pub fn health_counts() -> serde_json::Value {
    let reg = registry();
    let total = reg.len();
    let running = reg.values().filter(|ep| !ep.is_terminal()).count();
    let attached = reg.values().filter(|ep| ep.attached > 0).count();
    serde_json::json!({
        "running": running,
        "retained": total - running,
        "attached": attached,
    })
}

/// The [`EventSink`] the agent loop writes through: every event lands in the replay log
/// and the live fan-out at once. Unlike the old channel sink there is no such thing as
/// "nobody is listening, drop it" — the log is always listening.
pub struct RegistrySink {
    episode_id: String,
}

impl RegistrySink {
    pub fn new(episode_id: impl Into<String>) -> Self {
        Self {
            episode_id: episode_id.into(),
        }
    }
}

impl EventSink for RegistrySink {
    fn emit(&self, event: ChatEvent) {
        publish(&self.episode_id, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: usize = 1 << 20;

    fn drain(rx: &mut broadcast::Receiver<Arc<Frame>>) -> Vec<u64> {
        let mut seqs = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            seqs.push(frame.seq);
        }
        seqs
    }

    #[test]
    fn ids_are_unguessable_and_unique() {
        let a = new_episode_id();
        let b = new_episode_id();
        assert_ne!(a, b);
        // 128 bits of hex after the prefix — never a predictable counter.
        assert_eq!(a.len(), "ep-".len() + 32);
        assert!(
            a.strip_prefix("ep-")
                .unwrap()
                .bytes()
                .all(|c| c.is_ascii_hexdigit())
        );
    }

    #[test]
    fn replay_and_live_meet_exactly_once() {
        let (id, _cancel) = open(CAP);
        publish(&id, ChatEvent::Answer { text: "a".into() });
        publish(&id, ChatEvent::Answer { text: "b".into() });
        // Attach having already seen seq 1: replay must start at 2, and the live tail
        // must carry only what is published afterwards.
        let mut att = attach(&id, 1).expect("attach");
        assert_eq!(
            att.replay.iter().map(|f| f.seq).collect::<Vec<_>>(),
            vec![2],
            "replay must resume immediately after the client's last seq"
        );
        publish(&id, ChatEvent::Answer { text: "c".into() });
        assert_eq!(drain(&mut att.live), vec![3], "no gap, no duplicate");
    }

    #[test]
    fn attach_to_unknown_episode_is_refused() {
        let err = attach("ep-does-not-exist", 0).expect_err("must refuse");
        assert_eq!(err.body()["code"], ErrorCode::EpisodeUnknown.code());
    }

    #[test]
    fn resume_below_the_trimmed_floor_is_refused_not_holed() {
        // A cap of one small frame forces the log to trim as it grows.
        let (id, _cancel) = open(8);
        for _ in 0..4 {
            publish(
                &id,
                ChatEvent::Answer {
                    text: "xxxxxxxx".into(),
                },
            );
        }
        let err = attach(&id, 0).expect_err("a resume inside the lost range must be refused");
        assert_eq!(err.body()["code"], ErrorCode::EpisodeReplayTruncated.code());
        // The recent tail is still perfectly servable.
        attach(&id, 3).expect("a resume above the floor still attaches");
    }

    #[test]
    fn terminal_event_marks_finished_and_disarms_cancel() {
        let (id, _cancel) = open(CAP);
        publish(&id, ChatEvent::Done);
        assert_eq!(
            cancel(&id, CancelReason::UserRequest),
            CancelOutcome::AlreadyFinished
        );
        let att = attach(&id, 0).expect("a finished episode is still replayable");
        assert!(att.replay.last().expect("frame").terminal);
    }

    #[test]
    fn cancel_is_delivered_once_then_reports_finished() {
        let (id, mut cancel_rx) = open(CAP);
        assert_eq!(
            cancel(&id, CancelReason::UserRequest),
            CancelOutcome::Signalled
        );
        assert_eq!(cancel_rx.try_recv(), Ok(CancelReason::UserRequest));
        assert_eq!(
            cancel(&id, CancelReason::UserRequest),
            CancelOutcome::AlreadyFinished
        );
        assert_eq!(
            cancel("ep-nope", CancelReason::UserRequest),
            CancelOutcome::Unknown
        );
    }

    #[test]
    fn detaching_starts_the_abandonment_clock_and_reattaching_stops_it() {
        let (id, _cancel) = open(CAP);
        let att = attach(&id, 0).expect("attach");
        assert!(registry().get(&id).expect("episode").idle_since.is_none());
        drop(att);
        assert!(
            registry().get(&id).expect("episode").idle_since.is_some(),
            "dropping the attachment must start the clock"
        );
        let _att2 = attach(&id, 0).expect("reattach");
        assert!(
            registry().get(&id).expect("episode").idle_since.is_none(),
            "reattaching must clear the clock"
        );
    }

    #[test]
    fn reaper_cancels_an_abandoned_running_episode_but_spares_an_attached_one() {
        let (abandoned, mut abandoned_rx) = open(CAP);
        let (held, mut held_rx) = open(CAP);
        let _attachment = attach(&held, 0).expect("attach");
        // Zero grace: every unattached running episode is abandoned right now.
        reap(
            Duration::ZERO,
            Duration::from_secs(600),
            Duration::from_secs(3600),
        );
        assert_eq!(abandoned_rx.try_recv(), Ok(CancelReason::ClientGone));
        assert!(
            held_rx.try_recv().is_err(),
            "an attached episode must never be reaped"
        );
        assert_eq!(
            cancel(&abandoned, CancelReason::UserRequest),
            CancelOutcome::AlreadyFinished
        );
        drop(_attachment);
    }

    #[test]
    fn reaper_evicts_a_finished_episode_only_after_retention() {
        let (id, _cancel) = open(CAP);
        publish(&id, ChatEvent::Done);
        reap(
            Duration::ZERO,
            Duration::from_secs(600),
            Duration::from_secs(3600),
        );
        assert!(attach(&id, 0).is_ok(), "still inside the retention window");
        reap(Duration::ZERO, Duration::ZERO, Duration::from_secs(3600));
        let err = attach(&id, 0).expect_err("evicted after retention");
        assert_eq!(err.body()["code"], ErrorCode::EpisodeUnknown.code());
    }

    #[test]
    fn reaper_evicts_a_running_episode_that_outlived_every_budget() {
        let (id, _cancel) = open(CAP);
        // leak_after = 0: this registration is older than any budget by definition.
        let (_cancelled, evicted) = reap(
            Duration::from_secs(600),
            Duration::from_secs(600),
            Duration::ZERO,
        );
        assert!(evicted >= 1);
        assert_eq!(
            cancel(&id, CancelReason::UserRequest),
            CancelOutcome::Unknown
        );
    }
}

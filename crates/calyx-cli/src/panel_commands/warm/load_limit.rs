use super::load_progress::append_shared_progress;
use super::probe::run_progress_record;
use super::*;

impl WarmLoadLimit {
    pub(super) fn new(max_load_secs: u64) -> Self {
        Self {
            started: Instant::now(),
            max_load_secs,
        }
    }

    pub(super) fn recv<T>(&self, rx: &mpsc::Receiver<T>, wait: WarmLoadWait<'_>) -> CliResult<T> {
        if self.max_load_secs == 0 {
            return rx.recv().map_err(|_| {
                CliError::from(CalyxError::lens_unreachable(format!(
                    "panel warm worker channel closed during {phase}; completed={completed}/{total}",
                    phase = wait.phase,
                    completed = wait.completed,
                    total = wait.total,
                )))
            });
        }
        match self.remaining() {
            Some(remaining) if !remaining.is_zero() => rx.recv_timeout(remaining).map_err(|err| {
                match err {
                    mpsc::RecvTimeoutError::Timeout => self.timeout_error(&wait),
                    mpsc::RecvTimeoutError::Disconnected => {
                        CliError::from(CalyxError::lens_unreachable(format!(
                            "panel warm worker channel closed during {phase}; completed={completed}/{total}",
                            phase = wait.phase,
                            completed = wait.completed,
                            total = wait.total,
                        )))
                    }
                }
            }),
            _ => Err(self.timeout_error(&wait)),
        }
    }

    fn remaining(&self) -> Option<Duration> {
        if self.max_load_secs == 0 {
            return None;
        }
        Duration::from_secs(self.max_load_secs).checked_sub(self.started.elapsed())
    }

    pub(super) fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    fn timeout_error(&self, wait: &WarmLoadWait<'_>) -> CliError {
        let elapsed_ms = self.elapsed_ms();
        let message = format!(
            "panel warm readiness exceeded {max}s during {phase}; completed={completed}/{total}; \
             load_parallelism={load_parallelism}; elapsed_ms={elapsed_ms}; all configured lenses \
             must prepare and complete warmup inference inside the global readiness deadline",
            max = self.max_load_secs,
            phase = wait.phase,
            completed = wait.completed,
            total = wait.total,
            load_parallelism = wait.load_parallelism,
        );
        let mut record = run_progress_record(wait.selector, "load_timeout");
        record.elapsed_ms = Some(elapsed_ms);
        record.lens_count = Some(wait.total);
        record.load_parallelism = Some(wait.load_parallelism);
        record.error_code = Some(WARM_TIMEOUT.to_string());
        record.error_message = Some(message.clone());
        record.remediation = Some(
            "increase bounded load parallelism only if VRAM remains under cap, optimize or replace \
             slow lenses, or start the resident warm service once"
                .to_string(),
        );
        let _ = append_shared_progress(wait.progress_log, &record);
        CliError::from(CalyxError {
            code: WARM_TIMEOUT,
            message,
            remediation: "increase bounded load parallelism only if VRAM remains under cap, optimize or replace slow lenses, or start the resident warm service once",
        })
    }
}

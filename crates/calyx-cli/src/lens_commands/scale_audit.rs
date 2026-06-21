mod flags;
mod measure;
mod model;
mod probe;
mod report;
mod runtime;
mod worker;

#[cfg(test)]
mod tests;

use calyx_core::CalyxError;

use self::model::Flags;
use self::report::{ProgressUpdate, build_report, write_progress, write_report};
use self::worker::{audit_lens_with_timeout, run_worker};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(crate) fn scale_audit(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    if flags.worker {
        return run_worker(&flags);
    }
    let mut lenses = Vec::with_capacity(flags.manifests.len());
    for (idx, manifest) in flags.manifests.iter().enumerate() {
        write_progress(
            &flags,
            idx,
            manifest,
            ProgressUpdate::new("lens_started", "spawn_worker"),
            &lenses,
        )?;
        let audit = audit_lens_with_timeout(&flags, idx, manifest, &lenses)?;
        lenses.push(audit);
        write_progress(
            &flags,
            idx,
            manifest,
            ProgressUpdate::new("lens_finished", "complete"),
            &lenses,
        )?;
    }
    let report = build_report(lenses, &flags);
    write_report(&flags.out, &report)?;
    print_json(&report)?;
    if report.accepted {
        Ok(())
    } else {
        Err(CliError::Calyx(CalyxError {
            code: "CALYX_LENS_SCALE_ROSTER_REJECTED",
            message: format!(
                "scale audit rejected {} lens/panel condition(s); report={}",
                report.rejected_count,
                flags.out.display()
            ),
            remediation: "replace rejected lenses, prove batch stability/provider placement, then rerun lens scale-audit",
        }))
    }
}

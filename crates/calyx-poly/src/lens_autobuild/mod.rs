//! Local lens auto-build admission from panel-sufficiency deficits (issue #110).

mod engine;
mod types;

pub use self::engine::{
    LensAutobuildRun, compute_lens_autobuild_report, read_lens_autobuild_report,
    require_lens_autobuild_admitted, run_lens_autobuild_report, write_lens_autobuild_report,
};
pub use self::types::{
    BuiltLensSpec, LensAutobuildReport, LensAutobuildRequest, LensAutobuildStatus,
    LensCandidateMeasurement, LensCandidateRejection, LensDeficit,
};

pub const LENS_AUTOBUILD_SCHEMA_VERSION: &str = "poly.lens_autobuild.v2";
pub const LENS_AUTOBUILD_ARTIFACT_KIND: &str = "poly_lens_autobuild";
pub const LENS_AUTOBUILD_REPORT_FILE: &str = "lens_autobuild_report.json";
pub const LENS_AUTOBUILD_MIN_GAIN_BITS: f32 = 0.05;

pub const ERR_LENS_AUTOBUILD_INVALID_REQUEST: &str = "CALYX_POLY_LENS_AUTOBUILD_INVALID_REQUEST";
pub const ERR_LENS_AUTOBUILD_NO_DEFICIT: &str = "CALYX_POLY_LENS_AUTOBUILD_NO_DEFICIT";
pub const ERR_LENS_AUTOBUILD_NO_CANDIDATES: &str = "CALYX_POLY_LENS_AUTOBUILD_NO_CANDIDATES";
pub const ERR_LENS_AUTOBUILD_NO_ADMISSIBLE: &str = "CALYX_POLY_LENS_AUTOBUILD_NO_ADMISSIBLE";
pub const ERR_LENS_AUTOBUILD_READBACK_MISMATCH: &str =
    "CALYX_POLY_LENS_AUTOBUILD_READBACK_MISMATCH";

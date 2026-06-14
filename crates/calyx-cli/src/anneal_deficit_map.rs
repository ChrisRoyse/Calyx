use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_anneal::{
    AnchorId, AssayAttribution, CALYX_ASSAY_UNAVAILABLE, DEFAULT_DEFICIT_THRESHOLD_BITS,
    DeficitLocalizer, DeficitLocalizerConfig, has_deficit, top_gap_description,
};
use calyx_core::{CalyxError, FixedClock, LensId, Modality, Result as CalyxResult};
use serde::Deserialize;
use serde_json::json;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = DeficitMapRequest::parse(args)?;
    let fixture_bytes = fs::read(&request.fixture).map_err(|error| {
        format!(
            "{CALYX_ASSAY_UNAVAILABLE}: read fixture {}: {error}",
            request.fixture.display()
        )
    })?;
    let fixture = serde_json::from_slice::<Fixture>(&fixture_bytes).map_err(|error| {
        format!(
            "{CALYX_ASSAY_UNAVAILABLE}: parse fixture {}: {error}",
            request.fixture.display()
        )
    })?;
    let assay = FixtureAssay::from_fixture(fixture).map_err(format_calyx_error)?;
    let anchor = AnchorId::new(request.anchor).map_err(format_calyx_error)?;
    let clock = FixedClock::new(assay.clock_ts);
    let config = DeficitLocalizerConfig {
        deficit_threshold_bits: request.threshold,
        ..DeficitLocalizerConfig::default()
    };
    let localizer = DeficitLocalizer::with_config(&clock, config).map_err(format_calyx_error)?;
    let map = localizer
        .localize(&assay, &anchor, &assay.panel)
        .map_err(format_calyx_error)?;
    let readback = json!({
        "source_of_truth": "fixture JSON bytes read from fixture_path; map recomputed by calyx anneal deficit-map",
        "fixture_path": request.fixture.display().to_string(),
        "fixture_len": fixture_bytes.len(),
        "fixture_blake3": blake3::hash(&fixture_bytes).to_hex().to_string(),
        "anchor": anchor.as_str(),
        "panel": assay.panel.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "threshold": request.threshold,
        "has_deficit": has_deficit(&map, request.threshold),
        "top_gap_description": top_gap_description(&map),
        "map": map,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct DeficitMapRequest {
    anchor: String,
    fixture: PathBuf,
    threshold: f64,
}

impl DeficitMapRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut anchor = None;
        let mut fixture = None;
        let mut threshold = DEFAULT_DEFICIT_THRESHOLD_BITS;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--anchor" => {
                    anchor = args.get(idx + 1).cloned();
                    idx += 2;
                }
                "--fixture" => {
                    fixture = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--threshold" => {
                    let raw = args
                        .get(idx + 1)
                        .ok_or_else(|| "--threshold requires a value".to_string())?;
                    threshold = raw
                        .parse::<f64>()
                        .map_err(|error| format!("invalid --threshold: {error}"))?;
                    idx += 2;
                }
                other => return Err(format!("unknown deficit-map arg: {other}")),
            }
        }
        Ok(Self {
            anchor: anchor.ok_or_else(|| "deficit-map requires --anchor".to_string())?,
            fixture: fixture.ok_or_else(|| "deficit-map requires --fixture".to_string())?,
            threshold,
        })
    }
}

#[derive(Deserialize)]
struct Fixture {
    #[serde(default)]
    clock_ts: u64,
    panel: Vec<LensId>,
    anchors: Vec<FixtureAnchor>,
}

#[derive(Deserialize)]
struct FixtureAnchor {
    anchor_id: AnchorId,
    entropy_h: f64,
    panel_sufficiency: f64,
    #[serde(default)]
    expected_modalities: Vec<Modality>,
    #[serde(default)]
    bits_per_lens: Vec<FixtureLensBits>,
}

#[derive(Deserialize)]
struct FixtureLensBits {
    lens_id: LensId,
    bits: f64,
    modality: Option<Modality>,
}

struct FixtureAssay {
    clock_ts: u64,
    panel: Vec<LensId>,
    anchors: BTreeMap<AnchorId, FixtureAnchor>,
    lens_modalities: BTreeMap<LensId, Modality>,
}

impl FixtureAssay {
    fn from_fixture(fixture: Fixture) -> CalyxResult<Self> {
        let mut anchors = BTreeMap::new();
        let mut lens_modalities = BTreeMap::new();
        for anchor in fixture.anchors {
            if anchor.anchor_id.as_str().trim().is_empty() {
                return Err(unavailable("fixture anchor_id must not be empty"));
            }
            for row in &anchor.bits_per_lens {
                let Some(modality) = row.modality else {
                    continue;
                };
                let existing = lens_modalities.insert(row.lens_id, modality);
                if existing.is_some_and(|existing| existing != modality) {
                    return Err(unavailable(format!(
                        "fixture lens {} has conflicting modalities",
                        row.lens_id
                    )));
                }
            }
            anchors.insert(anchor.anchor_id.clone(), anchor);
        }
        Ok(Self {
            clock_ts: fixture.clock_ts,
            panel: fixture.panel,
            anchors,
            lens_modalities,
        })
    }

    fn anchor(&self, anchor: &AnchorId) -> CalyxResult<&FixtureAnchor> {
        self.anchors
            .get(anchor)
            .ok_or_else(|| unavailable(format!("missing anchor {anchor} in fixture")))
    }
}

impl AssayAttribution for FixtureAssay {
    fn per_sensor_bits(&self, anchor: &AnchorId) -> CalyxResult<Vec<(LensId, f64)>> {
        Ok(self
            .anchor(anchor)?
            .bits_per_lens
            .iter()
            .map(|row| (row.lens_id, row.bits))
            .collect())
    }

    fn panel_sufficiency(&self, anchor: &AnchorId) -> CalyxResult<f64> {
        Ok(self.anchor(anchor)?.panel_sufficiency)
    }

    fn entropy(&self, anchor: &AnchorId) -> CalyxResult<f64> {
        Ok(self.anchor(anchor)?.entropy_h)
    }

    fn expected_modalities(&self, anchor: &AnchorId) -> CalyxResult<Vec<Modality>> {
        Ok(self.anchor(anchor)?.expected_modalities.clone())
    }

    fn lens_modality(&self, lens: &LensId) -> CalyxResult<Option<Modality>> {
        Ok(self.lens_modalities.get(lens).copied())
    }
}

fn unavailable(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_UNAVAILABLE,
        message: message.into(),
        remediation: "provide a complete Assay attribution fixture or live Assay readback",
    }
}

fn format_calyx_error(error: CalyxError) -> String {
    format!("{}: {} ({})", error.code, error.message, error.remediation)
}

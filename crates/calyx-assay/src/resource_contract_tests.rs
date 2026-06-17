use calyx_core::Placement;

use super::*;

#[test]
fn resource_pack_prefers_many_small_high_density_lenses() {
    let budget = PanelResourceBudget {
        max_vram_mb: 900.0,
        max_ram_mb: 10_000.0,
        max_ms_per_input: 30.0,
    };
    let report = pack_panel_by_density(
        &[
            candidate("big_low_density", 0.70, 900.0, 9.0),
            candidate("small_a", 0.25, 200.0, 3.0),
            candidate("small_b", 0.25, 200.0, 3.0),
            candidate("small_c", 0.25, 200.0, 3.0),
        ],
        budget,
    )
    .unwrap();

    let selected: Vec<_> = report
        .selected
        .iter()
        .map(|decision| decision.lens.as_str())
        .collect();
    assert_eq!(selected, vec!["small_a", "small_b", "small_c"]);
    assert!(report.total_signal_bits > 0.70);
    assert!(report.remaining.vram_mb >= 299.0);
    assert!(
        report
            .rejected
            .iter()
            .any(|decision| decision.lens == "big_low_density")
    );
}

#[test]
fn resource_admission_rejects_oversized_lens() {
    let budget = PanelResourceBudget {
        max_vram_mb: 256.0,
        max_ram_mb: 1024.0,
        max_ms_per_input: 5.0,
    };
    let error = admit_lens_with_usage(
        0.20,
        0.10,
        ResourceUsage {
            vram_mb: 512.0,
            ram_mb: 0.0,
            ms_per_input: 1.0,
        },
        Placement::Gpu,
        budget,
    )
    .unwrap_err();

    assert_eq!(error.code, CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED);
}

#[test]
fn invalid_resource_budget_fails_closed() {
    let error = pack_panel_by_density(
        &[candidate("small", 0.10, 1.0, 1.0)],
        PanelResourceBudget {
            max_vram_mb: f32::NAN,
            max_ram_mb: 1.0,
            max_ms_per_input: 1.0,
        },
    )
    .unwrap_err();

    assert_eq!(error.code, CALYX_ASSAY_INVALID_RESOURCE);
}

fn candidate(
    lens: &str,
    signal_bits: f32,
    vram_mb: f32,
    ms_per_input: f32,
) -> PanelAdmissionCandidate {
    PanelAdmissionCandidate {
        lens: lens.to_string(),
        signal_bits,
        max_pairwise_corr: 0.10,
        usage: ResourceUsage {
            vram_mb,
            ram_mb: 64.0,
            ms_per_input,
        },
        placement: if vram_mb <= 0.0 {
            Placement::Cpu
        } else {
            Placement::Gpu
        },
        resident: false,
    }
}

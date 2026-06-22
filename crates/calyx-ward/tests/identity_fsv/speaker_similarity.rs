#[path = "speaker_similarity/support.rs"]
mod support;

use std::env;
use std::fs;
use std::sync::Arc;

use calyx_core::{FixedClock, SlotId};
use calyx_ward::{
    NoveltyHandler, SpeakerLens, VaultSink, WAVLM_DIM, WAVLM_SAMPLE_RATE, guard_generate,
};
use serde_json::json;
use support::*;

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_SPEAKER_FSV_DIR"]
fn fsv_stage8_speaker_similarity_target_writes_readbacks() {
    let root = required_path_env("CALYX_WARD_SPEAKER_FSV_DIR");
    assert_empty_or_absent(&root);
    fs::create_dir_all(&root).expect("create FSV root");
    write_edge_readbacks(&root);

    let identity_dir = env_path("CALYX_WARD_IDENTITY_FIXTURE_DIR", DEFAULT_IDENTITY_DIR);
    let fixture_dir = env::var_os("CALYX_WARD_SPEAKER_FIXTURE_DIR")
        .map(Into::into)
        .unwrap_or_else(|| identity_dir.join(DEFAULT_SPEAKER_FIXTURE));
    let spec_path = fixture_dir.join("speaker_profile.json");
    let spec: SpeakerFixtureSpec = read_json(&spec_path);
    let speaker_slot = SlotId::new(spec.speaker_slot);
    let matched_path =
        resolve_fixture_path(&identity_dir, &fixture_dir, &spec.matched_speaker_file);
    let in_region_dir = resolve_fixture_path(&identity_dir, &fixture_dir, &spec.in_region_dir);
    let cross_dir = resolve_fixture_path(&identity_dir, &fixture_dir, &spec.cross_speaker_dir);
    let in_region_paths = pcm_file_paths(&in_region_dir, MIN_IN_REGION).expect("in-region paths");
    let cross_paths = pcm_file_paths(&cross_dir, MIN_CROSS).expect("cross-speaker paths");

    let model_path = env_path("CALYX_WARD_WAVLM_MODEL", DEFAULT_WAVLM_MODEL_PATH);
    let speaker_lens =
        SpeakerLens::new_with_provider_policy(&model_path, speaker_provider_policy())
            .expect("load real WavLM speaker lens");
    let matched_audio = read_pcm_f32le(&matched_path).expect("matched speaker audio");
    let matched_vec = speaker_lens
        .embed_speaker(&matched_audio, spec.sample_rate)
        .expect("matched speaker embedding");
    assert_eq!(matched_vec.len(), WAVLM_DIM);
    let profile = speaker_identity_profile(&spec, speaker_slot, matched_vec.clone(), &spec_path);
    let vault = FileVault::new(root.join("cross-reject-records"));
    let handler = NoveltyHandler::new(Arc::new(vault.clone()), Arc::new(FixedClock::new(CLOCK_TS)));
    let style_lens = UnusedStyleLens;

    let fixture_readback = write_json(
        &root,
        "fixture-readback.json",
        &json!({
            "identity_dir": &identity_dir,
            "fixture_dir": &fixture_dir,
            "profile_path": &spec_path,
            "profile_sha256": sha256_file_hex(&spec_path),
            "fixture_manifest_sha256": sha256_file_hex(&fixture_dir.join("SHA256SUMS.txt")),
            "matched_speaker_file": &matched_path,
            "matched_speaker_sha256": sha256_file_hex(&matched_path),
            "in_region_dir": &in_region_dir,
            "in_region_count": in_region_paths.len(),
            "cross_speaker_dir": &cross_dir,
            "cross_speaker_count": cross_paths.len(),
            "target_voice": &spec.target_voice,
            "cross_voice": &spec.cross_voice,
            "source": &spec.source,
            "espeak_version": &spec.espeak_version,
            "sample_rate": spec.sample_rate,
            "items": &spec.items,
            "speaker_model": &model_path,
            "speaker_model_sha256": sha256_file_hex(&model_path),
            "speaker_provider_policy": speaker_lens.provider_policy(),
            "speaker_input_names": speaker_lens.input_names(),
            "speaker_output_names": speaker_lens.output_names(),
        }),
    );
    let matched_readback = write_json(
        &root,
        "matched-speaker-readback.json",
        &json!({
            "dim": matched_vec.len(),
            "norm": norm(&matched_vec),
            "prefix": prefix(&matched_vec, 5),
            "speaker_slot": speaker_slot,
            "tau": spec.tau,
        }),
    );

    let mut scores = Vec::new();
    let mut per_sample = Vec::new();
    for (index, path) in in_region_paths.iter().enumerate() {
        let audio = read_pcm_f32le(path).expect("in-region audio");
        let embedding = speaker_embedding_for_generate(&speaker_lens, &audio, spec.sample_rate);
        let direct_cos = cosine(&embedding, &matched_vec);
        let output = guard_generate(
            &profile,
            &speaker_input(audio, spec.sample_rate, index as u8),
            &speaker_lens,
            &style_lens,
            &handler,
            false,
        )
        .expect("in-region guard_generate");
        let slot = accepted_slot(&output, speaker_slot);
        assert!((slot.cos - direct_cos).abs() <= 1.0e-4);
        scores.push(direct_cos);
        per_sample.push(json!({
            "name": path.file_name().unwrap().to_string_lossy(),
            "path": path,
            "sha256": sha256_file_hex(path),
            "direct_cos": direct_cos,
            "guard_slot": slot,
            "output": output,
        }));
    }
    let mean_cos = mean(&scores);
    assert!(
        mean_cos >= spec.target_mean_cos,
        "FAIL: mean speaker sim {:.6} < {:.6} target",
        mean_cos,
        spec.target_mean_cos
    );
    let sample_readback = write_json(&root, "per-sample-speaker-verdicts.json", &per_sample);
    let mean_readback = write_json(
        &root,
        "mean-speaker-sim-readback.json",
        &json!({
            "metric": "mean_wavlm_speaker_similarity",
            "count": scores.len(),
            "mean": mean_cos,
            "min": scores.iter().copied().fold(f32::INFINITY, f32::min),
            "max": scores.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            "target": spec.target_mean_cos,
            "pass": mean_cos >= spec.target_mean_cos,
        }),
    );

    let mut cross = Vec::new();
    for (index, path) in cross_paths.iter().enumerate() {
        let audio = read_pcm_f32le(path).expect("cross speaker audio");
        let embedding = speaker_embedding_for_generate(&speaker_lens, &audio, spec.sample_rate);
        let direct_cos = cosine(&embedding, &matched_vec);
        let output = guard_generate(
            &profile,
            &speaker_input(audio, spec.sample_rate, (index + 80) as u8),
            &speaker_lens,
            &style_lens,
            &handler,
            false,
        )
        .expect("cross speaker guard_generate");
        let verdict = rejected_verdict(&output);
        let slot = slot_verdict(verdict, speaker_slot).clone();
        assert!(!verdict.overall_pass);
        assert!(!slot.pass);
        assert!(slot.cos < slot.tau);
        cross.push(json!({
            "name": path.file_name().unwrap().to_string_lossy(),
            "path": path,
            "sha256": sha256_file_hex(path),
            "direct_cos": direct_cos,
            "guard_slot": slot,
            "output": output,
        }));
    }
    let records = vault.novel_records().expect("cross reject records");
    let cross_readback = write_json(
        &root,
        "cross-speaker-rejection-readback.json",
        &json!({
            "count": cross.len(),
            "all_overall_pass_false": true,
            "records": records,
            "samples": cross,
        }),
    );
    let stage8_readback = write_stage8_summary(&root, mean_cos, spec.target_mean_cos);
    write_manifest(
        &root,
        &[
            fixture_readback,
            matched_readback,
            sample_readback,
            mean_readback,
            cross_readback,
            stage8_readback,
        ],
    );

    println!("mean_wavlm_speaker_similarity: {mean_cos:.6}");
    println!("Stage 8 Ward exit: PASS");
}

fn speaker_embedding_for_generate(lens: &SpeakerLens, audio: &[f32], sample_rate: u32) -> Vec<f32> {
    let prepared = if sample_rate == WAVLM_SAMPLE_RATE {
        audio.to_vec()
    } else {
        resample_linear(audio, sample_rate, WAVLM_SAMPLE_RATE)
    };
    lens.embed_speaker(&prepared, WAVLM_SAMPLE_RATE)
        .expect("generation-path speaker embedding")
}

fn resample_linear(audio: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    let out_len = ((audio.len() as f64) * (out_rate as f64) / (in_rate as f64))
        .round()
        .max(1.0) as usize;
    if audio.len() == 1 {
        return vec![audio[0]; out_len];
    }
    let scale = in_rate as f64 / out_rate as f64;
    (0..out_len)
        .map(|idx| {
            let pos = idx as f64 * scale;
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(audio.len() - 1);
            let frac = (pos - lo as f64) as f32;
            audio[lo] * (1.0 - frac) + audio[hi] * frac
        })
        .collect()
}

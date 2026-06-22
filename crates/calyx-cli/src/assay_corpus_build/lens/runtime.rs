use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Lens, Placement};
use calyx_registry::{
    CandleLens, FastembedBgem3Lens, FastembedQwen3Lens, FastembedRerankerLens, FastembedSparseLens,
    LensRuntime, LensSpec as RegistryLensSpec, MultimodalAdapterLens, OnnxColbertLens, OnnxLens,
    StaticLookupLens, TeiHttpLens,
};

use super::BuildLens;
use super::algorithmic::algorithmic_lens;
use crate::lens_commands::support::{dim, runtime_name};

pub(super) fn build_lens(manifest: PathBuf, spec: RegistryLensSpec) -> Result<BuildLens, String> {
    let runtime = runtime_name(&spec.runtime).to_string();
    match spec.runtime.clone() {
        LensRuntime::Algorithmic { kind } => {
            let lens = algorithmic_lens(&spec, &kind)?;
            Ok(cpu_build_lens(manifest, spec, runtime, Box::new(lens), 0.0))
        }
        LensRuntime::Onnx { files, .. } => {
            let lens = OnnxLens::from_lens_spec(&spec).map_err(lens_error)?;
            gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
        }
        LensRuntime::OnnxColbert { files, .. } => {
            let lens = OnnxColbertLens::from_lens_spec(&spec).map_err(lens_error)?;
            gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
        }
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            ..
        } => {
            let lens = StaticLookupLens::from_lens_spec(&spec).map_err(lens_error)?;
            let files = vec![embeddings_file.clone(), tokenizer.clone()];
            let ram_mb = paths_mb(&files)?;
            Ok(cpu_build_lens(
                manifest,
                spec,
                runtime,
                Box::new(lens),
                ram_mb,
            ))
        }
        LensRuntime::MultimodalAdapter { files, .. } => {
            let lens = MultimodalAdapterLens::from_lens_spec(&spec).map_err(lens_error)?;
            if lens.provider().is_gpu() {
                gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
            } else {
                let ram_mb = paths_mb(&files)?;
                Ok(cpu_build_lens(
                    manifest,
                    spec,
                    runtime,
                    Box::new(lens),
                    ram_mb,
                ))
            }
        }
        LensRuntime::TeiHttp { endpoint } => {
            let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim(spec.output));
            Ok(BuildLens {
                name: spec.name.clone(),
                manifest,
                spec,
                runtime_name: runtime,
                lens: Box::new(lens),
                placement: Placement::Gpu,
                default_vram_mb: f32::NAN,
                default_ram_mb: 0.0,
            })
        }
        LensRuntime::CandleLocal { files, .. } => {
            let lens = CandleLens::from_lens_spec(&spec).map_err(lens_error)?;
            gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
        }
        LensRuntime::FastembedQwen3 { files, .. } => {
            let lens = FastembedQwen3Lens::from_lens_spec(&spec).map_err(lens_error)?;
            gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
        }
        LensRuntime::FastembedSparse { files, .. } => {
            let lens = FastembedSparseLens::from_lens_spec(&spec).map_err(lens_error)?;
            gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
        }
        LensRuntime::FastembedBgem3 { files, .. } => {
            let lens = FastembedBgem3Lens::from_lens_spec(&spec).map_err(lens_error)?;
            gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
        }
        LensRuntime::FastembedReranker { files, .. } => {
            let lens = FastembedRerankerLens::from_lens_spec(&spec).map_err(lens_error)?;
            gpu_build_lens(manifest, spec, runtime, Box::new(lens), &files)
        }
        other => Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_UNSUPPORTED_RUNTIME: lens={} runtime={}",
            spec.name,
            runtime_name(&other)
        )),
    }
}

fn cpu_build_lens(
    manifest: PathBuf,
    spec: RegistryLensSpec,
    runtime_name: String,
    lens: Box<dyn Lens>,
    ram_mb: f32,
) -> BuildLens {
    BuildLens {
        name: spec.name.clone(),
        manifest,
        spec,
        runtime_name,
        lens,
        placement: Placement::Cpu,
        default_vram_mb: 0.0,
        default_ram_mb: ram_mb,
    }
}

fn gpu_build_lens(
    manifest: PathBuf,
    spec: RegistryLensSpec,
    runtime_name: String,
    lens: Box<dyn Lens>,
    files: &[PathBuf],
) -> Result<BuildLens, String> {
    Ok(BuildLens {
        name: spec.name.clone(),
        manifest,
        spec,
        runtime_name,
        lens,
        placement: Placement::Gpu,
        default_vram_mb: paths_mb(files)?,
        default_ram_mb: 0.0,
    })
}

fn paths_mb(paths: &[PathBuf]) -> Result<f32, String> {
    let bytes = paths.iter().try_fold(0_u64, |acc, path| {
        Ok::<u64, String>(acc.saturating_add(file_len(path)?))
    })?;
    Ok(bytes as f32 / (1024.0 * 1024.0))
}

fn file_len(path: &Path) -> Result<u64, String> {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_FILE_IO: {}: {error}",
                path.display()
            )
        })
}

fn lens_error(error: calyx_core::CalyxError) -> String {
    format!("{}: {}", error.code, error.message)
}

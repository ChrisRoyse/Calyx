use std::fs;

use calyx_core::{Input, Modality};
use calyx_registry::{Registry, lens_spec_from_manifest_path, profile_lens};

use super::flags::Flags;
use super::support::register_manifest_runtime;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(crate) fn card(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    flags.reject_list_flags("calyx lens card")?;
    let manifest = flags
        .manifest
        .clone()
        .ok_or_else(|| CliError::usage("calyx lens card requires --manifest <path>"))?;
    let spec = lens_spec_from_manifest_path(&manifest)?;
    let catalog_lens_id = spec.lens_id();
    let probes = probes_for(&flags, spec.modality)?;
    let mut registry = Registry::new();
    let runtime_lens_id = register_manifest_runtime(&mut registry, spec)?;
    let mut card = profile_lens(&registry, runtime_lens_id, &probes)?;
    card.lens_id = catalog_lens_id;
    print_json(&card)
}

fn probes_for(flags: &Flags, modality: Modality) -> CliResult<Vec<calyx_registry::ProfileProbe>> {
    if flags.input.is_some() && flags.input_file.is_some() {
        return Err(CliError::usage(
            "calyx lens card accepts only one of --input or --input-file",
        ));
    }
    if let Some(input) = &flags.input {
        return Ok(vec![probe(modality, input.as_bytes().to_vec(), None)]);
    }
    if let Some(path) = &flags.input_file {
        return Ok(vec![probe(modality, fs::read(path)?, None)]);
    }
    Ok(default_probe_bytes(modality)
        .into_iter()
        .map(|(bytes, label)| probe(modality, bytes, Some(label)))
        .collect())
}

fn probe(
    modality: Modality,
    bytes: Vec<u8>,
    label: Option<&'static str>,
) -> calyx_registry::ProfileProbe {
    match label {
        Some(label) => calyx_registry::ProfileProbe::labeled(Input::new(modality, bytes), label),
        None => calyx_registry::ProfileProbe::new(Input::new(modality, bytes)),
    }
}

fn default_probe_bytes(modality: Modality) -> Vec<(Vec<u8>, &'static str)> {
    match modality {
        Modality::Image => vec![(PNG_1X1.to_vec(), "image"); 3],
        Modality::Audio => vec![(WAV_TINY.to_vec(), "audio"); 3],
        _ => vec![
            (
                b"Calyx graph retrieval and storage contracts".to_vec(),
                "storage",
            ),
            (
                b"Multilingual policy evidence with audit trails".to_vec(),
                "policy",
            ),
            (
                b"GPU vector panels compare independent lenses".to_vec(),
                "systems",
            ),
            (
                b"Database bytes are the source of truth".to_vec(),
                "storage",
            ),
            (
                b"Legal and scientific terms need separate axes".to_vec(),
                "policy",
            ),
            (
                b"Temporal controls walk as-of state separately".to_vec(),
                "systems",
            ),
        ],
    }
}

const PNG_1X1: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 12, 73, 68, 65, 84, 8, 215, 99, 248, 15, 4, 0, 9, 251, 3, 253,
    167, 89, 129, 219, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

const WAV_TINY: &[u8] = &[
    82, 73, 70, 70, 40, 0, 0, 0, 87, 65, 86, 69, 102, 109, 116, 32, 16, 0, 0, 0, 1, 0, 1, 0, 64,
    31, 0, 0, 128, 62, 0, 0, 2, 0, 16, 0, 100, 97, 116, 97, 4, 0, 0, 0, 0, 0, 0, 0,
];

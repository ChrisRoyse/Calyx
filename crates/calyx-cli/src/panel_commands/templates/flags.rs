use super::*;

impl Flags {
    pub(super) fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--name" => {
                    idx += 1;
                    flags.name = Some(value(args, idx, "--name")?.to_string());
                }
                "--notes" => {
                    idx += 1;
                    flags.notes = Some(value(args, idx, "--notes")?.to_string());
                }
                "--from" => {
                    idx += 1;
                    flags.from = Some(value(args, idx, "--from")?.to_string());
                }
                "--template" => {
                    idx += 1;
                    flags.template = Some(value(args, idx, "--template")?.to_string());
                }
                "--vault" => {
                    idx += 1;
                    flags.vault = Some(value(args, idx, "--vault")?.to_string());
                }
                "--all-current" => flags.all_current = true,
                "--modality" => {
                    idx += 1;
                    flags.modality = Some(parse_modality(value(args, idx, "--modality")?)?);
                }
                "--lens" => {
                    idx += 1;
                    flags.lenses.push(value(args, idx, "--lens")?.to_string());
                }
                "--card" => {
                    idx += 1;
                    flags.cards.push(value(args, idx, "--card")?.into());
                }
                "--card-dir" => {
                    idx += 1;
                    flags.card_dir = Some(value(args, idx, "--card-dir")?.into());
                }
                "--assay-card" => {
                    idx += 1;
                    flags.assay_card = Some(value(args, idx, "--assay-card")?.into());
                }
                "--a37-admission-card" => {
                    idx += 1;
                    flags.a37_admission_card =
                        Some(value(args, idx, "--a37-admission-card")?.into());
                }
                "--require-a37-gate" => flags.require_a37_gate = true,
                other => return Err(CliError::usage(format!("unexpected template flag {other}"))),
            }
            idx += 1;
        }
        Ok(flags)
    }
}

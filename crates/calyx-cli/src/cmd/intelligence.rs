mod abundance;
mod bits;
mod core;
mod guard;
mod kernel;
mod model;
mod parse;
mod propose;

pub(crate) use parse::{AbundanceArgs, BitsArgs, GuardArgs, KernelArgs, ProposeLensArgs};

use super::Subcommand;
use crate::error::CliResult;

pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Bits(args) => bits::command(args),
        Subcommand::Kernel(args) => kernel::command(args),
        Subcommand::Guard(args) => guard::command(args),
        Subcommand::Abundance(args) => abundance::command(args),
        Subcommand::ProposeLens(args) => propose::command(args),
        _ => unreachable!("non-intelligence command routed to intelligence module"),
    }
}

pub(crate) fn parse_bits(rest: &[String]) -> CliResult<Subcommand> {
    parse::parse_bits(rest)
}

pub(crate) fn parse_kernel(rest: &[String]) -> CliResult<Subcommand> {
    parse::parse_kernel(rest)
}

pub(crate) fn parse_guard(rest: &[String]) -> CliResult<Subcommand> {
    parse::parse_guard(rest)
}

pub(crate) fn parse_abundance(rest: &[String]) -> CliResult<Subcommand> {
    parse::parse_abundance(rest)
}

pub(crate) fn parse_propose_lens(rest: &[String]) -> CliResult<Subcommand> {
    parse::parse_propose_lens(rest)
}

#[cfg(test)]
pub(crate) use parse::{
    GuardCommand, abundance_tokens, bits_tokens, guard_tokens, kernel_tokens, propose_lens_tokens,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod calibration_fsv_support;

#[cfg(test)]
mod calibration_fsv_tests;

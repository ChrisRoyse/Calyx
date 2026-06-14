use std::fs;
use std::path::PathBuf;

use crate::anneal_propose_lens_fixture::{Fixture, execute_fixture};

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = Request::parse(args)?;
    let fixture_bytes = fs::read(&request.fixture)
        .map_err(|error| format!("read fixture {}: {error}", request.fixture.display()))?;
    let fixture = serde_json::from_slice::<Fixture>(&fixture_bytes)
        .map_err(|error| format!("parse fixture {}: {error}", request.fixture.display()))?;
    let readback = execute_fixture(&request.fixture, fixture, &fixture_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct Request {
    fixture: PathBuf,
}

impl Request {
    fn parse(args: &[String]) -> Result<Self, String> {
        match args {
            [fixture_flag, fixture] if fixture_flag == "--fixture" => Ok(Self {
                fixture: PathBuf::from(fixture),
            }),
            _ => Err("propose-lens-run requires --fixture <json>".to_string()),
        }
    }
}

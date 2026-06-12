use std::path::Path;
use std::str::FromStr;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SystemClock, VaultId};
use calyx_oracle::{DomainId, oracle_self_consistency};
use serde_json::json;

pub(crate) fn readback_oracle_self_consistency(args: &[String]) -> Result<(), String> {
    match args {
        [vault_flag, vault, domain_flag, domain, vault_id_flag, vault_id, salt_flag, salt]
            if vault_flag == "--vault"
                && domain_flag == "--domain"
                && vault_id_flag == "--vault-id"
                && salt_flag == "--salt" =>
        {
            let vault_id =
                VaultId::from_str(vault_id).map_err(|error| format!("invalid --vault-id: {error}"))?;
            let vault = AsterVault::new_durable(
                Path::new(vault),
                vault_id,
                salt.as_bytes().to_vec(),
                VaultOptions::default(),
            )
            .map_err(|error| error.to_string())?;
            let clock = SystemClock;
            match oracle_self_consistency(&vault, DomainId::from(domain.clone()), &clock) {
                Ok(result) => {
                    vault.flush()
                        .map_err(|error| format!("flush oracle ledger row: {error}"))?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
                    );
                    Ok(())
                }
                Err(error) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "domain": domain,
                            "error_code": error.code(),
                            "error": error.to_string(),
                        }))
                        .map_err(|error| error.to_string())?
                    );
                    Err(error.to_string())
                }
            }
        }
        _ => Err("usage: calyx readback oracle_self_consistency --vault <dir> --domain <domain> --vault-id <id> --salt <s>".to_string()),
    }
}

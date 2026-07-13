#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;

    use calyx_gatebrokerd::broker_error::BrokerError;
    use calyx_gatebrokerd::daemon::{Broker, load_config};
    use calyx_gatebrokerd::logging::{self, Level};
    use calyx_gatebrokerd::protocol::StableCode;
    use calyx_gatebrokerd::transport::SeqpacketListener;

    const EX_USAGE: i32 = 64;
    const EX_CONFIG: i32 = 78;
    const EX_FATAL: i32 = 125;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Command {
        VerifyConfig,
        Serve,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Invocation {
        command: Command,
        config: PathBuf,
    }

    struct Failure {
        error: BrokerError,
        event: &'static str,
        exit_code: i32,
    }

    pub(super) fn entry() -> ! {
        // This is an invariant of the broker rather than a configurable
        // preference. It also protects any SQLite side files created during
        // early startup before their exact policy is re-verified.
        unsafe { libc::umask(0o077) };
        install_panic_hook();

        let exit_code = match run(std::env::args_os()) {
            Ok(()) => 0,
            Err(failure) => {
                if failure.exit_code == EX_FATAL {
                    failure.error.clone().fatal().log(failure.event);
                } else {
                    failure.error.log(failure.event);
                }
                failure.exit_code
            }
        };
        std::process::exit(exit_code)
    }

    fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), Failure> {
        let invocation = parse_invocation(arguments).map_err(|error| Failure {
            error,
            event: "daemon_cli_invalid",
            exit_code: EX_USAGE,
        })?;

        let config_path = invocation.config;
        let config = load_config(&config_path)
            .map_err(|error| startup_failure(error, "config_load_failed"))?;

        match invocation.command {
            Command::VerifyConfig => {
                Broker::verify(config)
                    .map_err(|error| startup_failure(error, "config_verification_failed"))?;
                info(
                    "config_verified",
                    "CALYX_GATEBROKER_CONFIG_VERIFIED",
                    "configuration, authority paths, journal, and recovery state verified",
                    [("config_path", config_path.display().to_string())],
                );
                Ok(())
            }
            Command::Serve => {
                // Validate the inherited kernel object before journal recovery
                // can mutate durable state. Broker::serve repeats the pathname
                // check immediately before accepting requests.
                let socket_path = config.raw().socket_path.clone();
                let listener = SeqpacketListener::from_systemd().map_err(|error| Failure {
                    error: BrokerError::new(
                        StableCode::CapabilityUnavailable,
                        format!("systemd socket activation contract failed: {error}"),
                        "Start calyx-gatebrokerd.service through calyx-gatebrokerd.socket and verify LISTEN_PID, LISTEN_FDS=1, LISTEN_FDNAMES=control, SOCK_SEQPACKET, and PassCredentials=no.",
                    )
                    .context("socket_path", socket_path.display().to_string())
                    .fatal(),
                    event: "socket_activation_failed",
                    exit_code: EX_FATAL,
                })?;
                listener.verify_bound_path(&socket_path).map_err(|error| Failure {
                    error: BrokerError::new(
                        StableCode::CapabilityUnavailable,
                        format!("activated control socket validation failed: {error}"),
                        "Repair the socket unit so its one named sequential-packet descriptor is bound to the configured control path.",
                    )
                    .context("socket_path", socket_path.display().to_string())
                    .fatal(),
                    event: "socket_path_verification_failed",
                    exit_code: EX_FATAL,
                })?;

                let broker = Broker::open(config)
                    .map_err(|error| startup_failure(error, "broker_startup_failed"))?;
                info(
                    "broker_ready",
                    "CALYX_GATEBROKER_READY",
                    "broker sources of truth are verified and the activated control socket is ready",
                    [
                        ("config_path", config_path.display().to_string()),
                        ("socket_path", socket_path.display().to_string()),
                    ],
                );
                broker.serve(listener).map_err(|error| Failure {
                    error: error.fatal(),
                    event: "broker_serve_failed",
                    exit_code: EX_FATAL,
                })
            }
        }
    }

    fn parse_invocation(
        arguments: impl IntoIterator<Item = OsString>,
    ) -> Result<Invocation, BrokerError> {
        let values: Vec<OsString> = arguments.into_iter().collect();
        if values.len() != 4 || values[2] != OsStr::new("--config") || values[3].is_empty() {
            return Err(usage_error());
        }
        let command = match values[1].to_str() {
            Some("verify-config") => Command::VerifyConfig,
            Some("serve") => Command::Serve,
            _ => return Err(usage_error()),
        };
        Ok(Invocation {
            command,
            config: PathBuf::from(&values[3]),
        })
    }

    fn usage_error() -> BrokerError {
        BrokerError::new(
            StableCode::InvalidRequest,
            "invalid daemon invocation",
            "Use exactly one of: calyx-gatebrokerd verify-config --config ABSOLUTE_PATH; calyx-gatebrokerd serve --config ABSOLUTE_PATH.",
        )
    }

    fn startup_failure(error: BrokerError, event: &'static str) -> Failure {
        let exit_code = if error.code == StableCode::ConfigInvalid {
            EX_CONFIG
        } else {
            EX_FATAL
        };
        Failure {
            error,
            event,
            exit_code,
        }
    }

    fn info<const N: usize>(event: &str, code: &str, message: &str, context: [(&str, String); N]) {
        let context = context
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>();
        logging::emit(
            Level::Info,
            event,
            code,
            message,
            "No action is required.",
            &context,
        );
    }

    fn install_panic_hook() {
        std::panic::set_hook(Box::new(|panic| {
            let mut context = BTreeMap::new();
            if let Some(location) = panic.location() {
                context.insert("source_file".into(), location.file().into());
                context.insert("source_line".into(), location.line().to_string());
            }
            logging::emit(
                Level::Critical,
                "broker_panic",
                "CALYX_GATEBROKER_INTERNAL",
                "the privileged broker encountered an internal invariant failure",
                "Inspect the preceding structured events and all journal/filesystem/cgroup sources of truth before restarting.",
                &context,
            );
            // Continuing after a panic could leave authority in a partially
            // observed state. Terminate the whole broker, including panics in
            // connection threads, and let the next start run durable recovery.
            std::process::exit(EX_FATAL);
        }));
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn args(values: &[&str]) -> Vec<OsString> {
            values.iter().map(OsString::from).collect()
        }

        #[test]
        fn accepts_only_the_two_service_contract_invocations() {
            assert_eq!(
                parse_invocation(args(&[
                    "calyx-gatebrokerd",
                    "verify-config",
                    "--config",
                    "/etc/calyx-gatebrokerd/config.toml",
                ]))
                .unwrap(),
                Invocation {
                    command: Command::VerifyConfig,
                    config: "/etc/calyx-gatebrokerd/config.toml".into(),
                }
            );
            assert_eq!(
                parse_invocation(args(&[
                    "calyx-gatebrokerd",
                    "serve",
                    "--config",
                    "/etc/calyx-gatebrokerd/config.toml",
                ]))
                .unwrap()
                .command,
                Command::Serve
            );
        }

        #[test]
        fn rejects_missing_reordered_unknown_and_extra_arguments() {
            for invalid in [
                args(&["calyx-gatebrokerd"]),
                args(&[
                    "calyx-gatebrokerd",
                    "serve",
                    "/etc/calyx-gatebrokerd/config.toml",
                    "--config",
                ]),
                args(&[
                    "calyx-gatebrokerd",
                    "unknown",
                    "--config",
                    "/etc/calyx-gatebrokerd/config.toml",
                ]),
                args(&[
                    "calyx-gatebrokerd",
                    "serve",
                    "--config",
                    "/etc/calyx-gatebrokerd/config.toml",
                    "extra",
                ]),
            ] {
                assert_eq!(
                    parse_invocation(invalid).unwrap_err().code,
                    StableCode::InvalidRequest
                );
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn main() {
    linux::entry()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    use calyx_gatebrokerd::broker_error::BrokerError;
    use calyx_gatebrokerd::protocol::StableCode;

    BrokerError::new(
        StableCode::CapabilityUnavailable,
        "calyx-gatebrokerd is supported only on Linux",
        "Install and run the checked-in Linux system service on a cgroup v2 host.",
    )
    .fatal()
    .log("broker_platform_unsupported");
    std::process::exit(125);
}

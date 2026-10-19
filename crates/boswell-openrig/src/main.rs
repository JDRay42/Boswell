//! `boswell-openrig` — hook commands and the event bridge.
//!
//! ```text
//! boswell-openrig hook session-start       # reads the hook payload on stdin
//! boswell-openrig hook user-prompt-submit
//! boswell-openrig bridge
//! ```

use boswell_openrig::boswell::Boswell;
use boswell_openrig::bridge::{self, BridgeConfig};
use boswell_openrig::config::{list, state_dir, BoswellConfig};
use boswell_openrig::hook::{self, HookEvent};
use std::io::Read;
use std::time::Duration;

const USAGE: &str = "usage: boswell-openrig hook <session-start|user-prompt-submit>\n       \
                     boswell-openrig bridge";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime always builds");

    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["hook", name] => {
            let Some(event) = HookEvent::parse(name) else {
                eprintln!("{}", USAGE);
                std::process::exit(2);
            };
            let mut raw = String::new();
            let _ = std::io::stdin().read_to_string(&mut raw);
            let payload = serde_json::from_str(&raw).unwrap_or_default();
            // A hook never fails the prompt: whatever happens, exit 0.
            if let Some(out) = runtime.block_on(hook::run(event, &payload, &env)) {
                println!("{}", out);
            }
        }
        ["bridge"] => {
            let Some(config) = BoswellConfig::from_env(&env) else {
                eprintln!("boswell-openrig bridge: set BOSWELL_URL and BOSWELL_TOKEN (or BOSWELL_TOKEN_FILE)");
                std::process::exit(1);
            };
            let boswell = match Boswell::new(config, Duration::from_secs(10)) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("boswell-openrig bridge: {}", e);
                    std::process::exit(1);
                }
            };
            let outbound = list(env("BOSWELL_BRIDGE_CHANGES"));
            let bridge_config = BridgeConfig {
                openrig_url: env("OPENRIG_URL")
                    .unwrap_or_else(|| "http://127.0.0.1:7433".to_string())
                    .trim_end_matches('/')
                    .to_string(),
                state_path: state_dir(&env).join("bridge.json"),
                poll_every: Duration::from_secs(
                    env("BOSWELL_BRIDGE_POLL_SECS")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(15),
                ),
                outbound_changes: if outbound.is_empty() {
                    vec![
                        "contradicted".to_string(),
                        "climbed".to_string(),
                        "superseded".to_string(),
                    ]
                } else {
                    outbound
                },
            };
            eprintln!(
                "boswell-openrig bridge: OpenRig {} <-> Boswell; state in {}",
                bridge_config.openrig_url,
                bridge_config.state_path.display()
            );
            runtime.block_on(bridge::run(boswell, bridge_config));
        }
        _ => {
            eprintln!("{}", USAGE);
            std::process::exit(2);
        }
    }
}

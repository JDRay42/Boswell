//! The files in `examples/openrig/` parse, and name only what exists: hook
//! commands this binary answers, and the MCP server the fragments start.

use boswell_openrig::hook::HookEvent;
use serde_json::Value;
use std::path::PathBuf;

fn example(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/openrig")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
}

fn json(rel: &str) -> Value {
    serde_json::from_str(&example(rel)).unwrap_or_else(|e| panic!("{}: {}", rel, e))
}

#[test]
fn every_hook_command_is_one_this_binary_answers() {
    for file in [
        "plugin/boswell/hooks/claude.json",
        "plugin/boswell/hooks/codex.json",
    ] {
        let hooks = json(file);
        let mut seen = 0;
        for (event, entries) in hooks["hooks"].as_object().unwrap() {
            for entry in entries.as_array().unwrap() {
                for h in entry["hooks"].as_array().unwrap() {
                    let command = h["command"].as_str().unwrap();
                    let name = command
                        .strip_prefix("boswell-openrig hook ")
                        .unwrap_or_else(|| panic!("{} runs {:?}", file, command));
                    assert!(
                        HookEvent::parse(name).is_some(),
                        "{} {} names unknown hook {}",
                        file,
                        event,
                        name
                    );
                    seen += 1;
                }
            }
        }
        assert_eq!(seen, 2, "{}: one SessionStart, one UserPromptSubmit", file);
    }
}

#[test]
fn the_plugin_manifests_point_at_their_hooks() {
    assert_eq!(
        json("plugin/boswell/.claude-plugin/plugin.json")["hooks"],
        "./hooks/claude.json"
    );
    assert_eq!(
        json("plugin/boswell/.codex-plugin/plugin.json")["hooks"],
        "./hooks/codex.json"
    );
}

#[test]
fn the_runtime_fragments_start_boswell_mcp_and_approve_it() {
    let mcp = json("runtime/claude-mcp.fragment.json");
    assert_eq!(mcp["mcpServers"]["boswell"]["command"], "boswell-mcp");
    let settings = json("runtime/claude-settings.fragment.json");
    assert_eq!(settings["enabledMcpjsonServers"][0], "boswell");

    let codex: toml::Value =
        toml::from_str(&example("runtime/codex-config.fragment.toml")).unwrap();
    assert_eq!(
        codex["mcp_servers"]["boswell"]["command"].as_str(),
        Some("boswell-mcp")
    );
}

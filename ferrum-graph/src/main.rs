use std::{collections::HashMap, env, fs, process};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Config {
    entry: Option<String>,
    #[serde(default)]
    sensors: HashMap<String, SensorCfg>,
    #[serde(default)]
    transformers: HashMap<String, TransformerCfg>,
    #[serde(default)]
    hooks: HashMap<String, HookCfg>,
    #[serde(default)]
    terminators: HashMap<String, TerminatorCfg>,
    #[serde(default)]
    rules: Vec<RuleCfg>,
}

#[derive(Debug, Deserialize)]
struct HookCfg {
    plugin: String,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SensorCfg {
    plugin: String,
}

#[derive(Debug, Deserialize)]
struct TransformerCfg {
    plugin: String,
    #[serde(flatten)]
    extra: toml::Table,
}

#[derive(Debug, Deserialize)]
struct TerminatorCfg {
    plugin: String,
    #[serde(flatten)]
    extra: toml::Table,
}

#[derive(Debug, Deserialize)]
struct RuleCfg {
    id: String,
    threshold: u8,
    input: String,
    if_action: Option<String>,
    else_action: Option<String>,
    #[serde(default)]
    buffer_body: bool,
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn sensor_nid(name: &str) -> String {
    format!("s_{}", sanitize(name))
}

fn transformer_nid(name: &str) -> String {
    format!("t_{}", sanitize(name))
}

fn rule_nid(id: &str) -> String {
    format!("r_{}", sanitize(id))
}

fn hook_nid(name: &str) -> String {
    format!("h_{}", sanitize(name))
}

fn action_nid(key: &str) -> String {
    format!("a_{}", sanitize(key))
}

fn provider_nid(name: &str, sensors: &HashMap<String, SensorCfg>) -> String {
    if sensors.contains_key(name) {
        sensor_nid(name)
    } else {
        transformer_nid(name)
    }
}

fn is_hook(name: &str, hooks: &HashMap<String, HookCfg>) -> bool {
    hooks.contains_key(name)
}

/// Formats a terminator label from its plugin and extra TOML fields.
fn terminator_label(name: &str, cfg: &TerminatorCfg) -> String {
    let mut pairs: Vec<_> = cfg.extra.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str());
    let extras: Vec<String> = pairs
        .into_iter()
        .map(|(k, v)| {
            // Display strings without surrounding TOML quotes so DOT doesn't break.
            let val = match v {
                toml::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            format!("{k}={val}")
        })
        .collect();
    if extras.is_empty() {
        format!("{name}\\n({})", cfg.plugin)
    } else {
        format!("{name}\\n({})\\n{}", cfg.plugin, extras.join(", "))
    }
}

/// Returns true if `name` refers to a rule (by id).
fn is_rule(name: &str, rules: &[RuleCfg]) -> bool {
    rules.iter().any(|r| r.id == name)
}

/// Ensures a styled terminator action node exists, emitting it if not yet seen.
/// Returns the node id.
fn ensure_action_node(
    name: &str,
    terminators: &HashMap<String, TerminatorCfg>,
    emitted_actions: &mut HashMap<String, String>,
    out: &mut String,
) -> String {
    let nid = action_nid(name);
    if !emitted_actions.contains_key(name) {
        let label_txt = match name {
            "block" => "block\\n(HTTP 403)".to_string(),
            "pass" | "bypass" => "pass\\n(allow)".to_string(),
            n => terminators
                .get(n)
                .map(|t| terminator_label(n, t))
                .unwrap_or_else(|| n.to_string()),
        };
        emitted_actions.insert(name.to_string(), label_txt.clone());
        let fill = if name.contains("block") || name.contains("deny") || name.contains("reject") {
            "#f1948a"
        } else {
            "#a9dfbf"
        };
        out.push_str(&format!(
            "  {nid} [label=\"{label_txt}\", shape=box3d, style=filled, fillcolor=\"{fill}\"];\n"
        ));
    }
    nid
}

/// Emits a single if/else edge from a rule diamond, creating action nodes as needed.
#[allow(clippy::too_many_arguments)]
fn emit_action_edge(
    from_nid: &str,
    action: Option<&String>,
    label: &str,
    color: &str,
    penwidth: &str,
    rules: &[RuleCfg],
    hooks: &HashMap<String, HookCfg>,
    terminators: &HashMap<String, TerminatorCfg>,
    emitted_actions: &mut HashMap<String, String>,
    need_final_pass: &mut bool,
    out: &mut String,
) {
    let target_nid = match action {
        None => {
            *need_final_pass = true;
            out.push_str(&format!(
                "  {from_nid} -> final_pass [label=\"{label}\", color=\"{color}\" penwidth={penwidth}];\n",
            ));
            return;
        }
        Some(name) if is_rule(name, rules) => rule_nid(name),
        Some(name) if is_hook(name, hooks) => hook_nid(name),
        Some(name) => ensure_action_node(name, terminators, emitted_actions, out),
    };
    out.push_str(&format!(
        "  {from_nid} -> {target_nid} [label=\"{label}\", color=\"{color}\", penwidth={penwidth}];\n"
    ));
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: ferrum-graph <config.toml>");
        process::exit(1);
    });

    let content = fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("error reading '{path}': {e}");
        process::exit(1);
    });

    let cfg: Config = toml::from_str(&content).unwrap_or_else(|e| {
        eprintln!("error parsing '{path}': {e}");
        process::exit(1);
    });

    let mut out = String::new();
    out.push_str("digraph ferrum {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [fontname=\"Helvetica\", fontsize=11];\n");
    out.push_str("  edge [fontsize=9, color=\"#555555\"];\n\n");

    // ── Sensors ───────────────────────────────────────────────────────────────
    let mut sensor_names: Vec<_> = cfg.sensors.keys().collect();
    sensor_names.sort();
    for name in &sensor_names {
        let s = &cfg.sensors[*name];
        out.push_str(&format!(
            "  {} [label=\"{name}\\n({})\", shape=ellipse, style=filled, fillcolor=\"#aed6f1\"];\n",
            sensor_nid(name),
            s.plugin,
        ));
    }
    out.push('\n');

    // ── Transformers ──────────────────────────────────────────────────────────
    if !cfg.transformers.is_empty() {
        let mut tnames: Vec<_> = cfg.transformers.keys().collect();
        tnames.sort();
        for name in &tnames {
            let t = &cfg.transformers[*name];
            out.push_str(&format!(
                "  {} [label=\"{name}\\n({})\", shape=box, style=filled, fillcolor=\"#fdebd0\"];\n",
                transformer_nid(name),
                t.plugin,
            ));
        }
        out.push('\n');

        // Transformer input edges — handle both array `inputs` and table `inputs`
        for (name, t) in &cfg.transformers {
            let dst = transformer_nid(name);
            if let Some(toml::Value::Table(table)) = t.extra.get("inputs") {
                // weighted_sum style: inputs = { sensor = weight }
                let mut pairs: Vec<_> = table.iter().collect();
                pairs.sort_by_key(|(k, _)| k.as_str());
                for (input, weight_val) in pairs {
                    let src = provider_nid(input, &cfg.sensors);
                    let weight = weight_val
                        .as_float()
                        .map(|f| format!("w={f:.2}"))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "  {src} -> {dst} [label=\"{weight}\", style=dashed, color=\"#d4ac0d\"];\n"
                    ));
                }
            } else if let Some(toml::Value::Array(arr)) = t.extra.get("inputs") {
                // or/and style: inputs = ["a", "b"]
                for v in arr {
                    if let Some(input) = v.as_str() {
                        let src = provider_nid(input, &cfg.sensors);
                        out.push_str(&format!(
                            "  {src} -> {dst} [style=dashed, color=\"#d4ac0d\"];\n"
                        ));
                    }
                }
            } else if let Some(toml::Value::String(input)) = t.extra.get("input") {
                // not style: input = "a"
                let src = provider_nid(input, &cfg.sensors);
                out.push_str(&format!(
                    "  {src} -> {dst} [style=dashed, color=\"#d4ac0d\"];\n"
                ));
            }
        }
        out.push('\n');
    }

    let mut emitted_actions: HashMap<String, String> = HashMap::new();
    let mut need_final_pass = false;

    // ── Hooks ─────────────────────────────────────────────────────────────────
    if !cfg.hooks.is_empty() {
        let mut hnames: Vec<_> = cfg.hooks.keys().collect();
        hnames.sort();
        for name in &hnames {
            let h = &cfg.hooks[*name];
            out.push_str(&format!(
                "  {} [label=\"{name}\\n({})\", shape=hexagon, style=filled, fillcolor=\"#d7bde2\"];\n",
                hook_nid(name),
                h.plugin,
            ));
        }
        out.push('\n');

        // Hook next edges — ensure terminator targets get a styled node too.
        let mut hnames: Vec<_> = cfg.hooks.keys().collect();
        hnames.sort();
        for name in &hnames {
            let h = &cfg.hooks[*name];
            if let Some(next) = &h.next {
                let src = hook_nid(name);
                let dst = if is_rule(next, &cfg.rules) {
                    rule_nid(next)
                } else if is_hook(next, &cfg.hooks) {
                    hook_nid(next)
                } else {
                    ensure_action_node(next, &cfg.terminators, &mut emitted_actions, &mut out)
                };
                out.push_str(&format!(
                    "  {src} -> {dst} [label=\"next\", style=dashed, color=\"#8e44ad\"];\n"
                ));
            }
        }
        out.push('\n');
    }

    // ── Request entry node ────────────────────────────────────────────────────
    out.push_str(
        "  request [label=\"request\", shape=circle, style=filled, fillcolor=\"#d5d8dc\"];\n\n",
    );

    // ── Rules as decision diamonds ────────────────────────────────────────────
    // Emit rule nodes first, then edges (so action nodes appear at correct positions)
    for (i, rule) in cfg.rules.iter().enumerate() {
        let buf = if rule.buffer_body {
            "\\nbuffers body"
        } else {
            ""
        };
        let rnid = rule_nid(&rule.id);
        out.push_str(&format!(
            "  {rnid} [label=\"[{n}] {id}\\nscore ≥ {t}?{buf}\", shape=diamond, style=\"filled,bold\", fillcolor=\"#d7bde2\"];\n",
            n = i + 1,
            id = rule.id,
            t = rule.threshold,
        ));
    }
    out.push('\n');

    for rule in &cfg.rules {
        let rnid = rule_nid(&rule.id);

        // input → rule diamond
        let input_nid = provider_nid(&rule.input, &cfg.sensors);
        out.push_str(&format!("  {input_nid} -> {rnid} [label=\"score\"];\n"));

        // if_action (yes branch)
        emit_action_edge(
            &rnid,
            rule.if_action.as_ref(),
            "yes",
            "#922b21",
            "2",
            &cfg.rules,
            &cfg.hooks,
            &cfg.terminators,
            &mut emitted_actions,
            &mut need_final_pass,
            &mut out,
        );

        // else_action (no branch)
        emit_action_edge(
            &rnid,
            rule.else_action.as_ref(),
            "no",
            "#555555",
            "1",
            &cfg.rules,
            &cfg.hooks,
            &cfg.terminators,
            &mut emitted_actions,
            &mut need_final_pass,
            &mut out,
        );

        out.push('\n');
    }

    if need_final_pass {
        out.push_str("  final_pass [label=\"pass\\n(allow)\", shape=box3d, style=filled, fillcolor=\"#a9dfbf\"];\n\n");
    }

    // ── Entry edge: request → entry point ────────────────────────────────────
    let entry_nid = match &cfg.entry {
        Some(name) if is_rule(name, &cfg.rules) => rule_nid(name),
        Some(name) if is_hook(name, &cfg.hooks) => hook_nid(name),
        Some(name) => action_nid(name),
        None => {
            if let Some(first) = cfg.rules.first() {
                rule_nid(&first.id)
            } else {
                "final_pass".to_string()
            }
        }
    };
    out.push_str(&format!("  request -> {entry_nid};\n"));

    out.push_str("}\n");
    print!("{out}");
}

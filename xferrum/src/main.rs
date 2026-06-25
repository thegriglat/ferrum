use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xferrum", about = "Build a custom Ferrum WAF binary")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a ferrum binary with the specified plugins
    Build(BuildArgs),
}

#[derive(Parser)]
struct BuildArgs {
    /// Plugin to include. Can be repeated.
    /// Formats:
    ///   github.com/owner/repo          — latest git HEAD
    ///   github.com/owner/repo@v1.2.3   — tagged release
    ///   ./local/path or /abs/path      — local path dependency
    ///   crate-name@1.0                 — crates.io version
    /// If omitted, xferrum scans ./plugins/ for local plugins automatically.
    #[arg(long = "plugin", value_name = "SPEC")]
    plugins: Vec<String>,

    /// Output path for the compiled binary
    #[arg(long, short, default_value = "./ferrum")]
    output: PathBuf,

    /// Build in release mode
    #[arg(long)]
    release: bool,

    /// Path to the ferrum library crate (defaults to auto-detect from current dir or FERRUM_PATH)
    #[arg(long, env = "FERRUM_PATH")]
    ferrum_path: Option<PathBuf>,
}

/// A resolved plugin source for Cargo.toml.
enum PluginSource {
    Path(PathBuf),
    Git { url: String, tag: Option<String> },
    Registry { version: String },
}

struct Plugin {
    /// Cargo package name (with dashes), e.g. "sensor-ip"
    name: String,
    source: PluginSource,
}

fn parse_plugin_spec(spec: &str, base_dir: &Path) -> Result<Plugin> {
    // Local path: starts with ./ ../ or /
    if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
        let path = base_dir
            .join(spec)
            .canonicalize()
            .with_context(|| format!("cannot resolve path: {spec}"))?;
        let name = cargo_name_from_path(&path)?;
        return Ok(Plugin {
            name,
            source: PluginSource::Path(path),
        });
    }

    // GitHub: github.com/owner/repo[@tag]
    if spec.starts_with("github.com/") {
        let (repo_part, tag) = if let Some((r, t)) = spec.split_once('@') {
            (r, Some(t.to_string()))
        } else {
            (spec, None)
        };
        let url = format!("https://{repo_part}");
        let name = repo_part
            .split('/')
            .next_back()
            .context("invalid github spec")?
            .to_string();
        return Ok(Plugin {
            name,
            source: PluginSource::Git { url, tag },
        });
    }

    // crates.io: crate-name@version
    if let Some((name, version)) = spec.split_once('@') {
        return Ok(Plugin {
            name: name.to_string(),
            source: PluginSource::Registry {
                version: version.to_string(),
            },
        });
    }

    bail!("cannot parse plugin spec '{spec}': use github.com/owner/repo, ./path, or crate@version")
}

fn cargo_name_from_path(path: &Path) -> Result<String> {
    let cargo_toml = path.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml)
        .with_context(|| format!("cannot read {}", cargo_toml.display()))?;
    let doc: toml_edit::DocumentMut = content
        .parse()
        .with_context(|| format!("cannot parse {}", cargo_toml.display()))?;
    let name = doc["package"]["name"]
        .as_str()
        .with_context(|| format!("missing [package].name in {}", cargo_toml.display()))?
        .to_string();
    Ok(name)
}

/// Scan ./plugins/category/name/ directories (two levels deep).
fn scan_local_plugins(base_dir: &Path) -> Vec<Plugin> {
    let plugins_dir = base_dir.join("plugins");
    let mut found = Vec::new();
    let Ok(categories) = fs::read_dir(&plugins_dir) else {
        return found;
    };
    for cat in categories.flatten() {
        let Ok(ft) = cat.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(cat.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let path = entry.path();
            if !path.join("Cargo.toml").exists() {
                continue;
            }
            if let Ok(name) = cargo_name_from_path(&path) {
                found.push(Plugin {
                    name,
                    source: PluginSource::Path(path),
                });
            }
        }
    }
    found
}

/// Rust identifier from a crate name (dashes → underscores).
fn crate_ident(name: &str) -> String {
    name.replace('-', "_")
}

fn build(args: BuildArgs) -> Result<()> {
    let base_dir = std::env::current_dir().context("cannot get current dir")?;

    let ferrum_path = match args.ferrum_path {
        Some(p) => p.canonicalize().context("cannot resolve --ferrum-path")?,
        None => {
            // Try to find ferrum/ sibling or current dir
            let candidate = base_dir.join("ferrum");
            if candidate.join("Cargo.toml").exists() {
                candidate.canonicalize()?
            } else if base_dir.join("Cargo.toml").exists() {
                // We might be inside the ferrum workspace already
                base_dir.clone()
            } else {
                bail!("cannot find ferrum library. Use --ferrum-path or FERRUM_PATH env var");
            }
        }
    };

    // Collect plugins
    let plugins: Vec<Plugin> = if args.plugins.is_empty() {
        let found = scan_local_plugins(&base_dir);
        if found.is_empty() {
            bail!("no plugins found in ./plugins/ and no --plugin flags given");
        }
        eprintln!("Auto-discovered {} plugin(s):", found.len());
        for p in &found {
            eprintln!("  {}", p.name);
        }
        found
    } else {
        args.plugins
            .iter()
            .map(|s| parse_plugin_spec(s, &base_dir))
            .collect::<Result<Vec<_>>>()?
    };

    // Build in a temp workspace
    let tmp = tempfile::TempDir::new().context("cannot create temp dir")?;
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir)?;

    // Generate Cargo.toml
    let mut cargo = format!(
        "[package]\nname = \"ferrum-custom\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nferrum = {{ path = \"{}\" }}\nclap = {{ version = \"4\", features = [\"derive\", \"env\"] }}\n",
        ferrum_path.display()
    );
    for plugin in &plugins {
        let dep = match &plugin.source {
            PluginSource::Path(p) => format!("{{ path = \"{}\" }}", p.display()),
            PluginSource::Git { url, tag: None } => format!("{{ git = \"{url}\" }}"),
            PluginSource::Git { url, tag: Some(t) } => {
                format!("{{ git = \"{url}\", tag = \"{t}\" }}")
            }
            PluginSource::Registry { version } => format!("{{ version = \"{version}\" }}"),
        };
        cargo.push_str(&format!("\"{}\" = {}\n", plugin.name, dep));
    }
    fs::write(tmp.path().join("Cargo.toml"), &cargo)?;

    // Generate src/main.rs
    let mut main_rs = String::new();
    for plugin in &plugins {
        main_rs.push_str(&format!("use {} as _;\n", crate_ident(&plugin.name)));
    }
    main_rs.push_str(concat!(
        "\nuse clap::Parser;\n\n",
        "#[derive(Parser)]\n",
        "#[command(name = \"ferrum\", about = \"Ferrum WAF\")]\n",
        "struct Args {\n",
        "    /// Path to the TOML config file\n",
        "    #[arg(long, default_value = \"ferrum.toml\", env = \"FERRUM_CONFIG\")]\n",
        "    config: String,\n",
        "}\n\n",
        "fn main() {\n",
        "    let args = Args::parse();\n",
        "    ferrum::run(&args.config);\n",
        "}\n"
    ));
    fs::write(src_dir.join("main.rs"), &main_rs)?;

    // Run cargo build
    let mut cmd = Command::new("cargo");
    cmd.arg("build").current_dir(tmp.path());
    if args.release {
        cmd.arg("--release");
    }
    let status = cmd.status().context("cargo build failed")?;
    if !status.success() {
        bail!("cargo build exited with {status}");
    }

    // Copy binary to output
    let profile = if args.release { "release" } else { "debug" };
    let bin = tmp
        .path()
        .join("target")
        .join(profile)
        .join("ferrum-custom");
    let output = if args.output.is_absolute() {
        args.output.clone()
    } else {
        base_dir.join(&args.output)
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&bin, &output)
        .with_context(|| format!("cannot copy {} to {}", bin.display(), output.display()))?;

    eprintln!("Binary written to {}", output.display());
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Build(args) => build(args),
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

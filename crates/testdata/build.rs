//! Generates metadata for vendored Game Boy test ROMs during the build.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct Manifest {
    #[allow(dead_code)]
    generated: Option<String>,
    source: Source,
    #[serde(rename = "rom")]
    roms: Vec<RomEntry>,
}

#[derive(Debug, Deserialize)]
struct Source {
    bundle: String,
    vendor: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RomEntry {
    suite: String,
    name: String,
    path: String,
    model: String,
    kind: String,
    expected: Expected,
    embed: bool,
}

#[derive(Debug, Deserialize)]
struct Expected {
    kind: String,
    value: Option<String>,
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn model_variant(model: &str) -> &'static str {
    match model {
        "dmg" | "DMG" => "Dmg",
        "cgb" | "CGB" => "Cgb",
        "sgb" | "SGB" => "Sgb",
        _ => "Unknown",
    }
}

fn kind_variant(kind: &str) -> &'static str {
    match kind {
        "cpu" | "CPU" => "Cpu",
        "timing" | "Timing" => "Timing",
        "visual" | "Visual" => "Visual",
        "audio" | "Audio" => "Audio",
        "serial" | "Serial" => "Serial",
        _ => "Unknown",
    }
}

fn expected_expr(expected: &Expected) -> String {
    match expected.kind.as_str() {
        "serial_ascii" => {
            let value = expected.value.as_deref().unwrap_or("Passed\\n");
            format!("Expected::SerialAscii({})", rust_string(value))
        }
        "mooneye_fib" => "Expected::MooneyeFib".to_owned(),
        "screenshot" => "Expected::Screenshot".to_owned(),
        _ => "Expected::Unknown".to_owned(),
    }
}

fn main() -> Result<()> {
    if env::var("GBX_SKIP_TESTROMS").is_ok() {
        println!("cargo:warning=Skipping test ROM vendoring (GBX_SKIP_TESTROMS set)");
        let out_dir = PathBuf::from(env::var("OUT_DIR")?);
        let rom_out_dir = out_dir.join("roms");
        fs::create_dir_all(&rom_out_dir)?;

        let generated_path = out_dir.join("generated.rs");
        let stub = r#"
use crate::types::{Expected, RomKind, RomMeta, RomModel};

pub(crate) const SOURCE_BUNDLE: &str = "skipped";
pub(crate) const SOURCE_VENDOR: &str = "skipped";
pub(crate) const SOURCE_URL: &str = "";

pub(crate) static ROMS: &[RomMeta] = &[];

#[cfg(feature = "embed")]
pub(crate) static EMBED_DATA: &[(&str, &[u8])] = &[];
"#;
        fs::write(&generated_path, stub)?;
        println!(
            "cargo:rustc-env=TESTDATA_DATA_DIR={}",
            rom_out_dir.to_string_lossy()
        );
        return Ok(());
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let manifest_path = manifest_dir.join("roms.index.toml");
    println!("cargo:rerun-if-changed={}", manifest_path.display());

    let manifest_str =
        fs::read_to_string(&manifest_path).with_context(|| format!("reading {manifest_path:?}"))?;
    let manifest: Manifest = toml::from_str(&manifest_str)?;

    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .context("unable to resolve workspace root")?
        .to_path_buf();

    let bundle_dir = workspace_root
        .join("third_party")
        .join("testroms")
        .join(&manifest.source.bundle);

    println!("cargo:rerun-if-changed={}", bundle_dir.display());

    if !bundle_dir.exists() {
        return Err(anyhow!(
            "vendor directory {bundle_dir:?} missing; run `devenv tasks run assets:testroms`"
        ));
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let rom_out_dir = out_dir.join("roms");
    fs::create_dir_all(&rom_out_dir)?;

    let mut generated = String::new();
    writeln!(
        &mut generated,
        "use crate::types::{{Expected, RomKind, RomMeta, RomModel}};"
    )?;
    writeln!(
        &mut generated,
        "pub(crate) const SOURCE_BUNDLE: &str = {};",
        rust_string(&manifest.source.bundle)
    )?;
    writeln!(
        &mut generated,
        "pub(crate) const SOURCE_VENDOR: &str = {};",
        rust_string(manifest.source.vendor.as_deref().unwrap_or("unknown"))
    )?;
    writeln!(
        &mut generated,
        "pub(crate) const SOURCE_URL: &str = {};",
        rust_string(manifest.source.url.as_deref().unwrap_or(""))
    )?;
    writeln!(&mut generated)?;
    generated.push_str("pub(crate) static ROMS: &[RomMeta] = &[\n");

    let mut embed_lines: Vec<String> = Vec::new();

    for rom in &manifest.roms {
        let source_file = bundle_dir.join(&rom.path);
        let rom_bytes =
            fs::read(&source_file).with_context(|| format!("reading ROM file {source_file:?}"))?;
        let sha = hex::encode(Sha256::digest(&rom_bytes));
        let size = rom_bytes.len() as u64;

        let dest = rom_out_dir.join(&rom.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, &rom_bytes).with_context(|| format!("copying ROM bytes to {dest:?}"))?;

        writeln!(
            &mut generated,
            "    RomMeta {{ suite: {}, name: {}, path: {}, model: RomModel::{}, kind: RomKind::{}, expected: {}, sha256: {}, size: {size}, embed: {} }},",
            rust_string(&rom.suite),
            rust_string(&rom.name),
            rust_string(&rom.path),
            model_variant(&rom.model),
            kind_variant(&rom.kind),
            expected_expr(&rom.expected),
            rust_string(&sha),
            rom.embed
        )?;

        if rom.embed {
            let embed_source = Path::new("../../third_party/testroms")
                .join(&manifest.source.bundle)
                .join(&rom.path);
            let embed_rel = embed_source.to_string_lossy().replace('\\', "/");
            embed_lines.push(format!(
                "    ({}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{embed_rel}\")).as_slice()),\n",
                rust_string(&rom.path)
            ));
        }
    }

    generated.push_str("];\n\n");
    generated.push_str("#[cfg(feature = \"embed\")]\n");
    generated.push_str("pub(crate) static EMBED_DATA: &[(&str, &[u8])] = &[\n");
    for line in embed_lines {
        generated.push_str(&line);
    }
    generated.push_str("];\n");

    let generated_path = out_dir.join("generated.rs");
    let mut file = fs::File::create(&generated_path)?;
    file.write_all(generated.as_bytes())?;

    println!(
        "cargo:rustc-env=TESTDATA_DATA_DIR={}",
        rom_out_dir.to_string_lossy()
    );

    Ok(())
}

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{env, fs, io::Read, path::Path};
use walkdir::WalkDir;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FactoryPreset {
    bank: String,
    slot: usize,
    name: String,
    normalized: String,
    tags: Vec<String>,
    fingerprint: String,
}

fn normalized_name(name: &str) -> String {
    name.trim_start_matches('_')
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = env::args()
        .nth(1)
        .ok_or("Factory source path is required")?;
    let output = env::args().nth(2).ok_or("Output path is required")?;
    let mut presets = Vec::new();
    for entry in WalkDir::new(source).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("dn2pst"))
        {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let bytes = stem.as_bytes();
        if bytes.len() < 5 {
            continue;
        }
        let bank = bytes[0].to_ascii_uppercase() as char;
        let slot = stem[1..4].parse::<usize>()?;
        let name = stem[5..].trim().to_string();
        let file = fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let manifest: serde_json::Value = {
            let mut entry = archive.by_name("manifest.json")?;
            let mut body = String::new();
            entry.read_to_string(&mut body)?;
            serde_json::from_str(&body)?
        };
        let payload_name = manifest
            .get("Payload")
            .and_then(|value| value.as_str())
            .ok_or("Missing payload")?;
        let mut payload = Vec::new();
        archive.by_name(payload_name)?.read_to_end(&mut payload)?;
        let tags = manifest
            .pointer("/MetaInfo/Tags")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .map(str::to_owned)
            .collect();
        presets.push(FactoryPreset {
            bank: bank.to_string(),
            slot,
            normalized: normalized_name(&name),
            name,
            tags,
            fingerprint: format!("{:x}", Sha256::digest(payload)),
        });
    }
    presets.sort_by(|a, b| a.bank.cmp(&b.bank).then(a.slot.cmp(&b.slot)));
    if let Some(parent) = Path::new(&output).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, serde_json::to_vec_pretty(&presets)?)?;
    Ok(())
}

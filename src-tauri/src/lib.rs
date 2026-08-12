use midir::{Ignore, MidiInput, MidiInputConnection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, State};
use walkdir::WalkDir;

const BANKS: &[char] = &['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H'];

#[derive(Default)]
struct MidiState {
    connection: Mutex<Option<MidiInputConnection<()>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MidiPort {
    index: usize,
    name: String,
    likely_digitone: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SysExReceipt {
    byte_count: usize,
    saved_path: String,
    received_at_ms: u128,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    banks_path: Option<String>,
    packs_path: Option<String>,
}

#[derive(Clone)]
struct Parsed {
    name: String,
    normalized: String,
    fingerprint: Option<String>,
    tags: Vec<String>,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Preset {
    bank: String,
    slot: usize,
    name: String,
    tags: Vec<String>,
    exact_packs: Vec<String>,
    name_only_packs: Vec<String>,
    duplicate_locations: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct Match {
    location: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Pack {
    name: String,
    total: usize,
    found: usize,
    exact: usize,
    name_only: usize,
    tags: BTreeMap<String, usize>,
    matches: Vec<Match>,
}

#[derive(Serialize)]
struct ScanResult {
    banks: BTreeMap<String, Vec<Preset>>,
    packs: Vec<Pack>,
    errors: Vec<String>,
}

fn settings_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|p| p.join("settings.json"))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn load_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    let path = settings_file(&app)?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    if let Some(root) = settings.banks_path.as_ref() {
        for bank in BANKS {
            fs::create_dir_all(Path::new(root).join(bank.to_string()))
                .map_err(|e| e.to_string())?;
        }
    }
    let path = settings_file(&app)?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(
        path,
        serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_midi_inputs() -> Result<Vec<MidiPort>, String> {
    let input = MidiInput::new("Digitone Presets discovery").map_err(|e| e.to_string())?;
    input
        .ports()
        .iter()
        .enumerate()
        .map(|(index, port)| {
            let name = input.port_name(port).map_err(|e| e.to_string())?;
            let lower = name.to_lowercase();
            Ok(MidiPort {
                index,
                likely_digitone: lower.contains("digitone") || lower.contains("elektron"),
                name,
            })
        })
        .collect()
}

#[tauri::command]
fn start_sysex_capture(
    app: tauri::AppHandle,
    state: State<'_, MidiState>,
    port_index: usize,
) -> Result<(), String> {
    let mut guard = state.connection.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Err("A MIDI input is already connected".into());
    }

    let mut input = MidiInput::new("Digitone Presets SysEx receiver").map_err(|e| e.to_string())?;
    input.ignore(Ignore::None);
    let ports = input.ports();
    let port = ports
        .get(port_index)
        .ok_or_else(|| "The selected MIDI port is no longer available".to_string())?;
    let port_name = input.port_name(port).map_err(|e| e.to_string())?;
    let dump_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("sysex-dumps");
    fs::create_dir_all(&dump_dir).map_err(|e| e.to_string())?;
    let event_app = app.clone();
    let mut message_buffer = Vec::<u8>::new();
    let mut receiving = false;

    let connection = input
        .connect(
            port,
            "digitone-presets-sysex-input",
            move |_timestamp, bytes, _| {
                for &byte in bytes {
                    if byte == 0xF0 {
                        message_buffer.clear();
                        receiving = true;
                    }
                    if receiving {
                        message_buffer.push(byte);
                    }
                    if receiving && byte == 0xF7 {
                        receiving = false;
                        let received_at_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|value| value.as_millis())
                            .unwrap_or_default();
                        let path = dump_dir.join(format!("digitone-{received_at_ms}.syx"));
                        let result = fs::write(&path, &message_buffer)
                            .map(|_| SysExReceipt {
                                byte_count: message_buffer.len(),
                                saved_path: path.to_string_lossy().into_owned(),
                                received_at_ms,
                            })
                            .map_err(|error| error.to_string());
                        let _ = event_app.emit("sysex-received", result);
                        message_buffer.clear();
                    }
                }
            },
            (),
        )
        .map_err(|e| format!("Could not connect to {port_name}: {e}"))?;
    *guard = Some(connection);
    Ok(())
}

#[tauri::command]
fn stop_sysex_capture(state: State<'_, MidiState>) -> Result<(), String> {
    let mut guard = state.connection.lock().map_err(|e| e.to_string())?;
    guard.take();
    Ok(())
}

fn normalized_stem(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or_default();
    let bytes = stem.as_bytes();
    let stripped = if bytes.len() >= 5
        && matches!(bytes[0].to_ascii_uppercase(), b'A'..=b'H')
        && bytes[1..4].iter().all(u8::is_ascii_digit)
        && bytes[4].is_ascii_whitespace()
    {
        &stem[5..]
    } else {
        stem
    };
    stripped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn parse_preset(path: &Path) -> Parsed {
    let normalized = normalized_stem(path);
    let name = normalized_stem(path)
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let result = (|| -> Result<(String, Vec<String>), String> {
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let manifest: serde_json::Value = {
            let mut entry = zip.by_name("manifest.json").map_err(|e| e.to_string())?;
            let mut body = String::new();
            entry.read_to_string(&mut body).map_err(|e| e.to_string())?;
            serde_json::from_str(&body).map_err(|e| e.to_string())?
        };
        let payload = manifest
            .get("Payload")
            .and_then(|v| v.as_str())
            .ok_or("manifest.json has no Payload")?;
        let mut bytes = Vec::new();
        zip.by_name(payload)
            .map_err(|e| e.to_string())?
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let tags = manifest
            .pointer("/MetaInfo/Tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok((format!("{:x}", Sha256::digest(bytes)), tags))
    })();
    match result {
        Ok((fingerprint, tags)) => Parsed {
            name,
            normalized,
            fingerprint: Some(fingerprint),
            tags,
            error: None,
        },
        Err(error) => Parsed {
            name,
            normalized,
            fingerprint: None,
            tags: vec![],
            error: Some(error),
        },
    }
}

fn preset_files(root: &Path, bank_mode: bool) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = if bank_mode {
        fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect()
    } else {
        WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .collect()
    };
    files.retain(|p| {
        p.is_file()
            && p.extension().and_then(|v| v.to_str()).is_some_and(|e| {
                e.eq_ignore_ascii_case("dn2pst") || (!bank_mode && e.eq_ignore_ascii_case("dnsnd"))
            })
    });
    files.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    files
}

#[tauri::command]
fn scan_library(banks_path: String, packs_path: String) -> Result<ScanResult, String> {
    let banks_root = Path::new(&banks_path);
    let packs_root = Path::new(&packs_path);
    if !banks_root.is_dir() || !packs_root.is_dir() {
        return Err("Both collection folders must exist".into());
    }
    let mut errors = vec![];
    let mut pack_presets: HashMap<String, Vec<Parsed>> = HashMap::new();
    let mut by_hash: HashMap<String, HashSet<String>> = HashMap::new();
    let mut by_name: HashMap<String, HashSet<String>> = HashMap::new();
    for entry in fs::read_dir(packs_root)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        if entry.path().is_dir() {
            pack_presets
                .entry(entry.file_name().to_string_lossy().into_owned())
                .or_default();
        }
    }
    for path in preset_files(packs_root, false) {
        let pack = path
            .strip_prefix(packs_root)
            .ok()
            .and_then(|p| p.components().next())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed = parse_preset(&path);
        if let Some(hash) = &parsed.fingerprint {
            by_hash
                .entry(hash.clone())
                .or_default()
                .insert(pack.clone());
        } else if let Some(e) = &parsed.error {
            errors.push(format!("{}: {e}", path.display()));
        }
        by_name
            .entry(parsed.normalized.clone())
            .or_default()
            .insert(pack.clone());
        pack_presets.entry(pack).or_default().push(parsed);
    }
    let mut parsed_banks: BTreeMap<String, Vec<Parsed>> = BTreeMap::new();
    for bank in BANKS {
        let key = bank.to_string();
        let rows = preset_files(&banks_root.join(&key), true)
            .into_iter()
            .map(|p| {
                let x = parse_preset(&p);
                if let Some(e) = &x.error {
                    errors.push(format!("{}: {e}", p.display()));
                }
                x
            })
            .collect();
        parsed_banks.insert(key, rows);
    }
    let mut duplicate_map: HashMap<String, Vec<String>> = HashMap::new();
    for (bank, rows) in &parsed_banks {
        for (i, p) in rows.iter().enumerate() {
            if let Some(h) = &p.fingerprint {
                duplicate_map
                    .entry(h.clone())
                    .or_default()
                    .push(format!("{bank}{:03}", i + 1));
            }
        }
    }
    let mut banks = BTreeMap::new();
    for (bank, rows) in &parsed_banks {
        let output = rows
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let exact: HashSet<String> = p
                    .fingerprint
                    .as_ref()
                    .and_then(|h| by_hash.get(h))
                    .cloned()
                    .unwrap_or_default();
                let mut exact_packs = exact.iter().cloned().collect::<Vec<_>>();
                exact_packs.sort();
                let mut name_only_packs = by_name
                    .get(&p.normalized)
                    .cloned()
                    .unwrap_or_default()
                    .difference(&exact)
                    .cloned()
                    .collect::<Vec<_>>();
                name_only_packs.sort();
                let location = format!("{bank}{:03}", i + 1);
                let duplicate_locations = p
                    .fingerprint
                    .as_ref()
                    .and_then(|h| duplicate_map.get(h))
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|v| v != &location)
                    .collect();
                Preset {
                    bank: bank.clone(),
                    slot: i + 1,
                    name: p.name.clone(),
                    tags: p.tags.clone(),
                    exact_packs,
                    name_only_packs,
                    duplicate_locations,
                    error: p.error.clone(),
                }
            })
            .collect();
        banks.insert(bank.clone(), output);
    }
    let all_bank = parsed_banks
        .iter()
        .flat_map(|(b, r)| r.iter().enumerate().map(move |(i, p)| (b, i, p)))
        .collect::<Vec<_>>();
    let backup_hashes = all_bank
        .iter()
        .filter_map(|(_, _, p)| p.fingerprint.clone())
        .collect::<HashSet<_>>();
    let backup_names = all_bank
        .iter()
        .map(|(_, _, p)| p.normalized.clone())
        .collect::<HashSet<_>>();
    let mut packs = pack_presets
        .into_iter()
        .map(|(name, presets)| {
            let exact = presets
                .iter()
                .filter(|p| {
                    p.fingerprint
                        .as_ref()
                        .is_some_and(|h| backup_hashes.contains(h))
                })
                .count();
            let name_only = presets
                .iter()
                .filter(|p| {
                    !p.fingerprint
                        .as_ref()
                        .is_some_and(|h| backup_hashes.contains(h))
                        && backup_names.contains(&p.normalized)
                })
                .count();
            let hashes = presets
                .iter()
                .filter_map(|p| p.fingerprint.clone())
                .collect::<HashSet<_>>();
            let names = presets
                .iter()
                .map(|p| p.normalized.clone())
                .collect::<HashSet<_>>();
            let mut tags = BTreeMap::new();
            for tag in presets.iter().flat_map(|p| &p.tags) {
                *tags.entry(tag.clone()).or_default() += 1;
            }
            let matches = all_bank
                .iter()
                .filter(|(_, _, p)| {
                    p.fingerprint.as_ref().is_some_and(|h| hashes.contains(h))
                        || names.contains(&p.normalized)
                })
                .map(|(b, i, p)| Match {
                    location: format!("{b}{:03}", i + 1),
                    name: p.name.clone(),
                })
                .collect();
            Pack {
                name,
                total: presets.len(),
                found: exact + name_only,
                exact,
                name_only,
                tags,
                matches,
            }
        })
        .collect::<Vec<_>>();
    packs.sort_by_key(|p| p.name.to_lowercase());
    Ok(ScanResult {
        banks,
        packs,
        errors,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(MidiState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            list_midi_inputs,
            start_sysex_capture,
            stop_sysex_capture,
            scan_library
        ])
        .run(tauri::generate_context!())
        .expect("error while running Digitone Presets")
}

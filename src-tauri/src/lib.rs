use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    time::Duration,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DevicePreset {
    slot: usize,
    name: String,
}

#[derive(Serialize)]
struct DeviceBank {
    bank: String,
    presets: Vec<DevicePreset>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceCatalog {
    device_name: String,
    banks: Vec<DeviceBank>,
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
fn list_midi_outputs() -> Result<Vec<MidiPort>, String> {
    let output = MidiOutput::new("Digitone Presets discovery").map_err(|e| e.to_string())?;
    output
        .ports()
        .iter()
        .enumerate()
        .map(|(index, port)| {
            let name = output.port_name(port).map_err(|e| e.to_string())?;
            let lower = name.to_lowercase();
            Ok(MidiPort {
                index,
                likely_digitone: lower.contains("digitone") || lower.contains("elektron"),
                name,
            })
        })
        .collect()
}

fn pack_rpc_request(transaction: u8, path: &str) -> Vec<u8> {
    let mut raw = vec![0x03, transaction & 0x7f, 0x00, 0x00, 0x53];
    raw.extend(path.as_bytes());
    raw.push(0);
    // DataList has two trailing 32-bit range arguments (offset and limit).
    raw.extend([0; 8]);
    let mut message = vec![0xf0, 0x00, 0x20, 0x3c, 0x10, 0x00, 0x00];
    for chunk in raw.chunks(7) {
        let mut high_bits = 0u8;
        for (index, byte) in chunk.iter().enumerate() {
            message.push(byte & 0x7f);
            high_bits |= ((byte >> 7) & 1) << index;
        }
        message.push(high_bits);
    }
    message.push(0xf7);
    message
}

fn unpack_rpc_response(message: &[u8]) -> Option<Vec<u8>> {
    if message.len() < 10
        || message[..7] != [0xf0, 0x00, 0x20, 0x3c, 0x10, 0x00, 0x24]
        || *message.last()? != 0xf7
    {
        return None;
    }
    let encoded = &message[7..message.len() - 1];
    let mut raw = Vec::new();
    for chunk in encoded.chunks(8) {
        let (&high_bits, data) = chunk.split_last()?;
        for (index, byte) in data.iter().enumerate() {
            raw.push(byte | (((high_bits >> index) & 1) << 7));
        }
    }
    Some(raw)
}

fn preset_names(raw: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut start = None;
    for (index, byte) in raw.iter().copied().chain(std::iter::once(0)).enumerate() {
        if (0x20..=0x7e).contains(&byte) {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            if index - begin >= 2 {
                let value = String::from_utf8_lossy(&raw[begin..index])
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    names.push(value);
                }
            }
        }
    }
    names
}

fn read_device_catalog_blocking(
    input_index: usize,
    output_index: usize,
) -> Result<DeviceCatalog, String> {
    let mut input = MidiInput::new("Digitone Presets catalog reader").map_err(|e| e.to_string())?;
    input.ignore(Ignore::None);
    let input_ports = input.ports();
    let input_port = input_ports
        .get(input_index)
        .ok_or_else(|| "The selected MIDI input is no longer available".to_string())?;
    let device_name = input.port_name(input_port).map_err(|e| e.to_string())?;
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let _connection = input
        .connect(
            input_port,
            "digitone-presets-catalog-input",
            move |_timestamp, message, _| {
                if message.starts_with(&[0xf0, 0x00, 0x20, 0x3c]) {
                    let _ = sender.send(message.to_vec());
                }
            },
            (),
        )
        .map_err(|e| e.to_string())?;

    let output = MidiOutput::new("Digitone Presets catalog reader").map_err(|e| e.to_string())?;
    let output_ports = output.ports();
    let output_port = output_ports
        .get(output_index)
        .ok_or_else(|| "The selected MIDI output is no longer available".to_string())?;
    let mut connection = output
        .connect(output_port, "digitone-presets-catalog-output")
        .map_err(|e| e.to_string())?;

    let mut banks = Vec::new();
    for (index, bank) in BANKS.iter().enumerate() {
        let transaction = 0x20 + index as u8;
        let path = format!("/soundbanks/{bank}");
        connection
            .send(&pack_rpc_request(transaction, &path))
            .map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let raw = loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let message = receiver
                .recv_timeout(remaining)
                .map_err(|_| format!("Timed out while reading bank {bank}"))?;
            if let Some(raw) = unpack_rpc_response(&message) {
                if raw.get(3) == Some(&transaction) && raw.get(4) == Some(&0x53) {
                    break raw;
                }
            }
        };
        let presets = preset_names(raw.get(6..).unwrap_or_default())
            .into_iter()
            .take(256)
            .enumerate()
            .map(|(slot, name)| DevicePreset {
                slot: slot + 1,
                name,
            })
            .collect();
        banks.push(DeviceBank {
            bank: bank.to_string(),
            presets,
        });
    }
    Ok(DeviceCatalog { device_name, banks })
}

#[tauri::command]
async fn read_device_catalog(
    state: State<'_, MidiState>,
    input_index: usize,
    output_index: usize,
) -> Result<DeviceCatalog, String> {
    if state
        .connection
        .lock()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Stop SysEx capture before reading the device catalog".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        read_device_catalog_blocking(input_index, output_index)
    })
    .await
    .map_err(|e| e.to_string())?
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

#[cfg(test)]
mod rpc_tests {
    use super::*;

    #[test]
    fn data_list_request_matches_transfer_capture() {
        let expected = "F0 00 20 3C 10 00 00 03 01 00 00 53 2F 73 00 6F 75 6E 64 62 61 6E 00 6B 73 2F 41 00 00 00 00 00 00 00 00 00 00 00 F7"
            .split_whitespace()
            .map(|value| u8::from_str_radix(value, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pack_rpc_request(1, "/soundbanks/A"), expected);
    }

    #[test]
    fn printable_runs_are_extracted_as_names() {
        let data = [0, 1, b'B', b'A', b'S', b'S', 0, 2, b'P', b'A', b'D', 0];
        assert_eq!(preset_names(&data), ["BASS", "PAD"]);
    }
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
            list_midi_outputs,
            start_sysex_capture,
            stop_sysex_capture,
            read_device_catalog,
            scan_library
        ])
        .run(tauri::generate_context!())
        .expect("error while running Digitone Presets")
}

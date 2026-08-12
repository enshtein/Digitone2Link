use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{Read, Write},
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedDevicePreset {
    bank: String,
    slot: usize,
    name: String,
    saved_path: String,
    byte_count: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncProgress {
    completed: usize,
    total: usize,
    percent: usize,
    bank: String,
    slot: usize,
    stage: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncResult {
    catalog: DeviceCatalog,
    saved: usize,
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
    slot: usize,
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
    pack_rpc_command(transaction, 0x53, &data_list_arguments(path))
}

fn data_list_arguments(path: &str) -> Vec<u8> {
    let mut arguments = path.as_bytes().to_vec();
    arguments.push(0);
    arguments.extend([0; 8]);
    arguments
}

fn pack_rpc_command(transaction: u8, command: u8, arguments: &[u8]) -> Vec<u8> {
    let mut raw = vec![0x00, transaction & 0x7f, 0x00, 0x00, command];
    raw.extend(arguments);
    let mut message = vec![0xf0, 0x00, 0x20, 0x3c, 0x10, 0x00, 0x20];
    let header_length = raw.len().min(7);
    message.extend(&raw[..header_length]);
    for chunk in raw[header_length..].chunks(7) {
        let mut high_bits = 0u8;
        for (index, byte) in chunk.iter().enumerate() {
            high_bits |= ((byte >> 7) & 1) << (6 - index);
        }
        message.push(high_bits);
        message.extend(chunk.iter().map(|byte| byte & 0x7f));
        message.extend(std::iter::repeat_n(0, 7 - chunk.len()));
    }
    message.push(0xf7);
    message
}

fn rpc_exchange(
    connection: &mut midir::MidiOutputConnection,
    receiver: &mpsc::Receiver<Vec<u8>>,
    transaction: u8,
    command: u8,
    arguments: &[u8],
) -> Result<Vec<u8>, String> {
    connection
        .send(&pack_rpc_command(transaction, command, arguments))
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let message = receiver
            .recv_timeout(remaining)
            .map_err(|_| format!("Timed out waiting for RPC command {command:#04x}"))?;
        if let Some(raw) = unpack_rpc_response(&message) {
            if raw.get(3) == Some(&(transaction & 0x7f)) && raw.get(4) == Some(&command) {
                // Digitone II needs a short recovery interval between file RPC jobs.
                std::thread::sleep(Duration::from_millis(25));
                return Ok(raw);
            }
        }
    }
}

fn read_device_file(
    connection: &mut midir::MidiOutputConnection,
    receiver: &mpsc::Receiver<Vec<u8>>,
    transaction: &mut u8,
    path: &str,
) -> Result<Vec<u8>, String> {
    fn ensure_file_response(response: &[u8], path: &str, operation: &str) -> Result<(), String> {
        if response.get(5) == Some(&0) {
            let detail = String::from_utf8_lossy(response.get(6..).unwrap_or_default())
                .trim_matches(char::from(0))
                .to_string();
            let hex = response
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            return Err(format!(
                "Device rejected {operation} for {path}: {detail} ({hex})"
            ));
        }
        Ok(())
    }

    let mut open_args = path.as_bytes().to_vec();
    open_args.push(0);
    open_args.extend(if path.ends_with("/.metadata") {
        [0, 0, 8, 0x80]
    } else {
        [0, 0, 8, 0]
    });
    let open = rpc_exchange(connection, receiver, *transaction, 0x54, &open_args)
        .map_err(|error| format!("{error} while opening {path}"))?;
    ensure_file_response(&open, path, "open")?;
    *transaction = transaction.wrapping_add(1) & 0x7f;
    let handle = open
        .get(6..10)
        .ok_or_else(|| format!("Invalid open response for {path}"))?
        .to_vec();

    let mut first_args = handle.clone();
    first_args.extend([0, 0, 0, 0]);
    let first = rpc_exchange(connection, receiver, *transaction, 0x55, &first_args)?;
    ensure_file_response(&first, path, "initial read")?;
    *transaction = transaction.wrapping_add(1) & 0x7f;

    let data = if first.len() > 27 {
        first
    } else {
        let mut data_args = handle.clone();
        data_args.extend([0, 0, 0, 1]);
        let response = rpc_exchange(connection, receiver, *transaction, 0x55, &data_args)?;
        ensure_file_response(&response, path, "continued read")?;
        *transaction = transaction.wrapping_add(1) & 0x7f;
        response
    };

    let _ = rpc_exchange(connection, receiver, *transaction, 0x56, &handle)?;
    *transaction = transaction.wrapping_add(1) & 0x7f;
    if data.len() < 27 {
        let response = data
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        return Err(format!(
            "Invalid data response for {path} ({} bytes): {response}",
            data.len()
        ));
    }
    Ok(data[27..].to_vec())
}

fn safe_file_name(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let cleaned = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start_matches('_')
        .trim()
        .to_string();
    if cleaned.is_empty() {
        "Untitled".into()
    } else {
        cleaned
    }
}

fn write_local_preset(
    root: &Path,
    bank: char,
    slot: usize,
    name: &str,
    metadata: &[u8],
    payload: &[u8],
) -> Result<PathBuf, String> {
    let folder = root.join(bank.to_string());
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!("{bank}{slot:03} {}.dn2pst", safe_file_name(name)));
    let file = fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let metadata_json: serde_json::Value = serde_json::from_slice(metadata)
        .unwrap_or_else(|_| serde_json::json!({ "rawByteCount": metadata.len() }));
    let tags = metadata_json
        .get("sound_tags")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let manifest = serde_json::json!({
        "FormatVersion": 1,
        "ProductType": "Digitone II Preset",
        "Payload": "preset.bin",
        "MetaInfo": {
            "Name": name,
            "Bank": bank.to_string(),
            "Slot": slot,
            "Tags": tags,
            "DeviceMetadata": metadata_json
        }
    });
    archive
        .start_file("manifest.json", options)
        .map_err(|e| e.to_string())?;
    archive
        .write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    archive
        .start_file("preset.bin", options)
        .map_err(|e| e.to_string())?;
    archive.write_all(payload).map_err(|e| e.to_string())?;
    archive.finish().map_err(|e| e.to_string())?;
    Ok(path)
}

fn unpack_rpc_response(message: &[u8]) -> Option<Vec<u8>> {
    if message.len() < 10
        || !(matches!(message[6], 0x24) || matches!(message[6] & 0x0f, 0x0c | 0x0d))
        || message[..6] != [0xf0, 0x00, 0x20, 0x3c, 0x10, 0x00]
        || *message.last()? != 0xf7
    {
        return None;
    }
    let encoded = &message[7..message.len() - 1];
    let header_length = encoded.len().min(7);
    let mut raw = encoded[..header_length].to_vec();
    for chunk in encoded[header_length..].chunks(8) {
        let (&high_bits, data) = chunk.split_first()?;
        for (index, byte) in data.iter().enumerate() {
            raw.push(byte | (((high_bits >> (6 - index)) & 1) << 7));
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

fn device_presets(raw: &[u8]) -> Vec<DevicePreset> {
    let mut presets = Vec::new();
    let mut name_start = 0;
    let mut marker = 1;
    while marker + 13 < raw.len() {
        if raw[marker - 1] == 0 && raw[marker] == 0 && raw[marker + 1] == 2 {
            let slot = u32::from_be_bytes([
                raw[marker + 2],
                raw[marker + 3],
                raw[marker + 4],
                raw[marker + 5],
            ]) as usize;
            if (1..=256).contains(&slot) {
                let start = name_start.min(marker - 1);
                let name = preset_names(&raw[start..marker - 1])
                    .into_iter()
                    .last()
                    .unwrap_or_default()
                    .trim_start_matches('_')
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    presets.push(DevicePreset { slot, name });
                }
                name_start = marker + 14;
            }
        }
        marker += 1;
    }
    presets
}

fn read_device_catalog_blocking(
    input_index: usize,
    output_index: usize,
) -> Result<DeviceCatalog, String> {
    read_device_catalog_with_progress(input_index, output_index, |_completed, _bank| {})
}

fn read_device_catalog_with_progress<F>(
    input_index: usize,
    output_index: usize,
    mut progress: F,
) -> Result<DeviceCatalog, String>
where
    F: FnMut(usize, char),
{
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

    // Transfer initializes the v2 RPC session before requesting DataList.
    for (transaction, command) in [(0x07, 0x01), (0x08, 0x02), (0x09, 0x01), (0x0a, 0x03)] {
        rpc_exchange(&mut connection, &receiver, transaction, command, &[])
            .map_err(|error| format!("Device handshake failed: {error}"))?;
    }

    let mut banks = Vec::new();
    for (index, bank) in BANKS.iter().enumerate() {
        progress(index, *bank);
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
        let parsed = device_presets(raw.get(6..).unwrap_or_default());
        let presets = if parsed.is_empty() {
            preset_names(raw.get(6..).unwrap_or_default())
                .into_iter()
                .take(256)
                .enumerate()
                .map(|(slot, name)| DevicePreset {
                    slot: slot + 1,
                    name,
                })
                .collect()
        } else {
            parsed
        };
        banks.push(DeviceBank {
            bank: bank.to_string(),
            presets,
        });
        progress(index + 1, *bank);
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

fn download_device_preset_blocking(
    input_index: usize,
    output_index: usize,
    bank: char,
    slot: usize,
    name: String,
    library_path: String,
) -> Result<SavedDevicePreset, String> {
    if !BANKS.contains(&bank) || !(1..=256).contains(&slot) {
        return Err("Invalid bank or preset slot".into());
    }
    let mut input = MidiInput::new("Digitone Presets file reader").map_err(|e| e.to_string())?;
    input.ignore(Ignore::None);
    let input_ports = input.ports();
    let input_port = input_ports
        .get(input_index)
        .ok_or_else(|| "The selected MIDI input is no longer available".to_string())?;
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let _input_connection = input
        .connect(
            input_port,
            "digitone-presets-file-input",
            move |_timestamp, message, _| {
                if message.starts_with(&[0xf0, 0x00, 0x20, 0x3c]) {
                    let _ = sender.send(message.to_vec());
                }
            },
            (),
        )
        .map_err(|e| e.to_string())?;
    let output = MidiOutput::new("Digitone Presets file reader").map_err(|e| e.to_string())?;
    let output_ports = output.ports();
    let output_port = output_ports
        .get(output_index)
        .ok_or_else(|| "The selected MIDI output is no longer available".to_string())?;
    let mut output_connection = output
        .connect(output_port, "digitone-presets-file-output")
        .map_err(|e| e.to_string())?;
    let mut transaction = 0x60;
    let base = format!("/soundbanks/{bank}/{slot}");
    let metadata = read_device_file(
        &mut output_connection,
        &receiver,
        &mut transaction,
        &format!("{base}/.metadata"),
    )?;
    let payload = read_device_file(&mut output_connection, &receiver, &mut transaction, &base)?;
    let path = write_local_preset(
        Path::new(&library_path),
        bank,
        slot,
        &name,
        &metadata,
        &payload,
    )?;
    Ok(SavedDevicePreset {
        bank: bank.to_string(),
        slot,
        name,
        saved_path: path.to_string_lossy().into_owned(),
        byte_count: payload.len(),
    })
}

#[tauri::command]
async fn download_device_preset(
    state: State<'_, MidiState>,
    input_index: usize,
    output_index: usize,
    bank: String,
    slot: usize,
    name: String,
    library_path: String,
) -> Result<SavedDevicePreset, String> {
    if state
        .connection
        .lock()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Stop SysEx capture before downloading a preset".into());
    }
    let bank = bank
        .chars()
        .next()
        .map(|value| value.to_ascii_uppercase())
        .ok_or("Bank is required")?;
    tauri::async_runtime::spawn_blocking(move || {
        download_device_preset_blocking(input_index, output_index, bank, slot, name, library_path)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn sync_device_presets_blocking(
    app: tauri::AppHandle,
    input_index: usize,
    output_index: usize,
    library_path: String,
) -> Result<SyncResult, String> {
    let progress_app = app.clone();
    let catalog =
        read_device_catalog_with_progress(input_index, output_index, move |completed, bank| {
            let _ = progress_app.emit(
                "sync-progress",
                SyncProgress {
                    completed,
                    total: BANKS.len(),
                    percent: completed * 10 / BANKS.len(),
                    bank: bank.to_string(),
                    slot: 0,
                    stage: "Reading device catalog".into(),
                },
            );
        })?;
    let total = catalog
        .banks
        .iter()
        .map(|bank| bank.presets.len())
        .sum::<usize>();
    let root = PathBuf::from(&library_path);
    let staging = root.join(".digitone-presets-sync");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let result = (|| -> Result<usize, String> {
        let mut input =
            MidiInput::new("Digitone Presets synchronizer").map_err(|e| e.to_string())?;
        input.ignore(Ignore::None);
        let input_ports = input.ports();
        let input_port = input_ports
            .get(input_index)
            .ok_or_else(|| "The selected MIDI input is no longer available".to_string())?;
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        let _input_connection = input
            .connect(
                input_port,
                "digitone-presets-sync-input",
                move |_timestamp, message, _| {
                    if message.starts_with(&[0xf0, 0x00, 0x20, 0x3c]) {
                        let _ = sender.send(message.to_vec());
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        let output = MidiOutput::new("Digitone Presets synchronizer").map_err(|e| e.to_string())?;
        let output_ports = output.ports();
        let output_port = output_ports
            .get(output_index)
            .ok_or_else(|| "The selected MIDI output is no longer available".to_string())?;
        let mut output_connection = output
            .connect(output_port, "digitone-presets-sync-output")
            .map_err(|e| e.to_string())?;
        for (transaction, command) in [(0x30, 0x01), (0x31, 0x02), (0x32, 0x01), (0x33, 0x03)] {
            rpc_exchange(&mut output_connection, &receiver, transaction, command, &[])
                .map_err(|error| format!("File session handshake failed: {error}"))?;
        }
        let mut transaction = 0x34;
        for path in ["/", "/soundbanks"] {
            rpc_exchange(
                &mut output_connection,
                &receiver,
                transaction,
                0x53,
                &data_list_arguments(path),
            )
            .map_err(|error| format!("File session setup failed for {path}: {error}"))?;
            transaction = transaction.wrapping_add(1) & 0x7f;
        }
        let mut completed = 0;
        for device_bank in &catalog.banks {
            let bank = device_bank.bank.chars().next().ok_or("Invalid bank name")?;
            let bank_path = format!("/soundbanks/{bank}");
            rpc_exchange(
                &mut output_connection,
                &receiver,
                transaction,
                0x53,
                &data_list_arguments(&bank_path),
            )
            .map_err(|error| format!("Could not prepare bank {bank}: {error}"))?;
            transaction = transaction.wrapping_add(1) & 0x7f;
            for preset in &device_bank.presets {
                let base = format!("/soundbanks/{bank}/{}", preset.slot);
                let _ = app.emit(
                    "sync-progress",
                    SyncProgress {
                        completed,
                        total,
                        percent: if total == 0 {
                            10
                        } else {
                            10 + completed * 90 / total
                        },
                        bank: device_bank.bank.clone(),
                        slot: preset.slot,
                        stage: "Reading from device".into(),
                    },
                );
                let metadata = read_device_file(
                    &mut output_connection,
                    &receiver,
                    &mut transaction,
                    &format!("{base}/.metadata"),
                )?;
                let payload =
                    read_device_file(&mut output_connection, &receiver, &mut transaction, &base)?;
                write_local_preset(
                    &staging,
                    bank,
                    preset.slot,
                    &preset.name,
                    &metadata,
                    &payload,
                )?;
                completed += 1;
            }
        }

        let _ = app.emit(
            "sync-progress",
            SyncProgress {
                completed,
                total,
                percent: 100,
                bank: String::new(),
                slot: 0,
                stage: "Replacing local library".into(),
            },
        );
        for bank in BANKS {
            let target = root.join(bank.to_string());
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            for entry in fs::read_dir(&target).map_err(|e| e.to_string())?.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.eq_ignore_ascii_case("dn2pst"))
                {
                    fs::remove_file(path).map_err(|e| e.to_string())?;
                }
            }
            let source = staging.join(bank.to_string());
            if source.is_dir() {
                for entry in fs::read_dir(source).map_err(|e| e.to_string())?.flatten() {
                    let source_path = entry.path();
                    fs::rename(&source_path, target.join(entry.file_name()))
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(completed)
    })();
    let _ = fs::remove_dir_all(&staging);
    let saved = result?;
    Ok(SyncResult { catalog, saved })
}

#[tauri::command]
async fn sync_device_presets(
    app: tauri::AppHandle,
    state: State<'_, MidiState>,
    input_index: usize,
    output_index: usize,
    library_path: String,
) -> Result<SyncResult, String> {
    if state
        .connection
        .lock()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Stop SysEx capture before synchronizing presets".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        sync_device_presets_blocking(app, input_index, output_index, library_path)
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
        .trim_start_matches('_')
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn parse_preset(path: &Path) -> Parsed {
    let slot = path
        .file_stem()
        .and_then(|value| value.to_str())
        .and_then(|value| value.get(1..4))
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();
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
            .filter(|tags| !tags.is_empty())
            .or_else(|| {
                manifest
                    .pointer("/MetaInfo/DeviceMetadata/sound_tags")
                    .and_then(|v| v.as_array())
            })
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
            slot,
            name,
            normalized,
            fingerprint: Some(fingerprint),
            tags,
            error: None,
        },
        Err(error) => Parsed {
            slot,
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
        for p in rows {
            if let Some(h) = &p.fingerprint {
                duplicate_map
                    .entry(h.clone())
                    .or_default()
                    .push(format!("{bank}{:03}", p.slot));
            }
        }
    }
    let mut banks = BTreeMap::new();
    for (bank, rows) in &parsed_banks {
        let output = rows
            .iter()
            .map(|p| {
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
                let location = format!("{bank}{:03}", p.slot);
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
                    slot: p.slot,
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
        let expected = "F0 00 20 3C 10 00 20 00 22 00 00 53 2F 73 00 6F 75 6E 64 62 61 6E 00 6B 73 2F 41 00 00 00 00 00 00 00 00 00 00 00 F7"
            .split_whitespace()
            .map(|value| u8::from_str_radix(value, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pack_rpc_request(0x22, "/soundbanks/A"), expected);
    }

    #[test]
    fn printable_runs_are_extracted_as_names() {
        let data = [0, 1, b'B', b'A', b'S', b'S', 0, 2, b'P', b'A', b'D', 0];
        assert_eq!(preset_names(&data), ["BASS", "PAD"]);
    }

    #[test]
    fn catalog_parser_keeps_real_slots_and_skips_empty_entries() {
        let mut data = b"BASS\0\0\x02\0\0\0\x0b\0\0\x01\x6c\0\x7e\x01\x01".to_vec();
        data.extend(b"\0\0\x02\0\0\0\x0c\0\0\x01\x6c\0\x7e\x01\x01");
        data.extend(b"PAD\0\0\x02\0\0\0\x0d\0\0\x01\x6c\0\x7e\x01\x01");
        let presets = device_presets(&data);
        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0].slot, 11);
        assert_eq!(presets[0].name, "BASS");
        assert_eq!(presets[1].slot, 13);
        assert_eq!(presets[1].name, "PAD");
    }

    #[test]
    fn v2_identity_response_accepts_variable_transport_length_bits() {
        let message = "F0 00 20 3C 10 00 4C 00 16 00 07 01 2B 16 00 01 02 03 04 06 07 09 00 50 52 51 53 54 55 56 00 57 58 59 5A 5B 5C 5D 00 5E 44 69 67 69 74 6F 00 6E 65 20 49 49 00 F7"
            .split_whitespace()
            .map(|value| u8::from_str_radix(value, 16).unwrap())
            .collect::<Vec<_>>();
        let raw = unpack_rpc_response(&message).expect("identity response must decode");
        assert_eq!(raw.get(3), Some(&0x07));
        assert_eq!(raw.get(4), Some(&0x01));
        assert!(raw.windows(11).any(|value| value == b"Digitone II"));
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
            download_device_preset,
            sync_device_presets,
            scan_library
        ])
        .run(tauri::generate_context!())
        .expect("error while running Digitone Presets")
}

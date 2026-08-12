import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  Check,
  ChevronRight,
  FolderOpen,
  Library,
  LoaderCircle,
  RefreshCw,
  Settings as SettingsIcon,
  Tags,
  Usb,
  X,
} from "lucide-react";
import type { DeviceCatalog, MidiPort, Pack, ScanResult, Settings } from "./types";

const BANKS = [..."ABCDEFGH"];
const views = [
  { id: "presets", label: "Device Presets", icon: Library },
  { id: "packs", label: "Sound Packs", icon: Archive },
  { id: "tags", label: "Tags", icon: Tags },
  { id: "settings", label: "Settings", icon: SettingsIcon },
] as const;

type View = (typeof views)[number]["id"];

export default function App() {
  const [view, setView] = useState<View>("presets");
  const [bank, setBank] = useState("A");
  const [settings, setSettings] = useState<Settings>({ banksPath: null, packsPath: null });
  const [data, setData] = useState<ScanResult | null>(null);
  const [catalog, setCatalog] = useState<DeviceCatalog | null>(null);
  const [midiInputs, setMidiInputs] = useState<MidiPort[]>([]);
  const [midiOutputs, setMidiOutputs] = useState<MidiPort[]>([]);
  const [selectedInput, setSelectedInput] = useState<number | null>(null);
  const [selectedOutput, setSelectedOutput] = useState<number | null>(null);
  const [discovering, setDiscovering] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [onboarding, setOnboarding] = useState(false);
  const [message, setMessage] = useState("Ready");
  const [selectedPack, setSelectedPack] = useState<Pack | null>(null);
  const [emptyOnly, setEmptyOnly] = useState(false);

  const foldersReady = Boolean(settings.banksPath && settings.packsPath);
  const selectedDevice = midiInputs.find((port) => port.index === selectedInput);
  const deviceTotal = catalog?.banks.reduce((sum, item) => sum + item.presets.length, 0) ?? 0;

  useEffect(() => {
    void initialize();
  }, []);

  useEffect(() => {
    if (foldersReady && !data) void scanLibrary();
  }, [foldersReady]);

  async function initialize() {
    try {
      const saved = await invoke<Settings>("load_settings");
      setSettings(saved);
      await refreshMidi(true);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setDiscovering(false);
    }
  }

  async function refreshMidi(showOnboarding = false) {
    try {
      const [inputs, outputs] = await Promise.all([
        invoke<MidiPort[]>("list_midi_inputs"),
        invoke<MidiPort[]>("list_midi_outputs"),
      ]);
      setMidiInputs(inputs);
      setMidiOutputs(outputs);
      const input = inputs.find((port) => port.likelyDigitone) ?? inputs[0];
      const output = outputs.find((port) => port.likelyDigitone) ?? outputs[0];
      setSelectedInput(input?.index ?? null);
      setSelectedOutput(output?.index ?? null);
      if (showOnboarding && input && output) setOnboarding(true);
      setMessage(input && output ? `${input.name} is available` : "No MIDI device found");
    } catch (error) {
      setMessage(`MIDI discovery failed: ${error}`);
    }
  }

  async function scanLibrary() {
    if (!settings.banksPath || !settings.packsPath) return;
    setMessage("Scanning local library…");
    try {
      const result = await invoke<ScanResult>("scan_library", {
        banksPath: settings.banksPath,
        packsPath: settings.packsPath,
      });
      setData(result);
      setMessage(`Local library · ${Object.values(result.banks).flat().length} presets`);
    } catch (error) {
      setMessage(`Local scan failed: ${error}`);
    }
  }

  async function syncDevice() {
    if (selectedInput === null || selectedOutput === null) return;
    if (!settings.banksPath) {
      setOnboarding(false);
      setView("settings");
      setMessage("Choose a local Preset Library folder before synchronization");
      return;
    }
    setSyncing(true);
    setMessage("Synchronizing preset catalog…");
    try {
      const result = await invoke<DeviceCatalog>("read_device_catalog", {
        inputIndex: selectedInput,
        outputIndex: selectedOutput,
      });
      setCatalog(result);
      setOnboarding(false);
      setView("presets");
      setMessage(`Synchronized catalog · ${result.banks.reduce((sum, item) => sum + item.presets.length, 0)} presets`);
    } catch (error) {
      setMessage(`Synchronization failed: ${error}`);
    } finally {
      setSyncing(false);
    }
  }

  async function chooseFolder(kind: "banks" | "packs") {
    const path = await open({
      directory: true,
      multiple: false,
      title: kind === "banks" ? "Select local preset library" : "Select sound packs folder",
    });
    if (!path) return;
    const next = { ...settings, [kind === "banks" ? "banksPath" : "packsPath"]: path as string };
    await invoke("save_settings", { settings: next });
    setSettings(next);
    setData(null);
  }

  const tagCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    Object.values(data?.banks ?? {}).flat().forEach((preset) =>
      preset.tags.forEach((tag) => { counts[tag] = (counts[tag] ?? 0) + 1; }),
    );
    return Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  }, [data]);

  const deviceRows = catalog?.banks.find((item) => item.bank === bank)?.presets ?? [];
  const localRows = data?.banks[bank] ?? [];

  return (
    <div className="min-h-screen bg-canvas pb-10 text-slate-100">
      <header className="topbar">
        <div className="flex min-w-0 items-center gap-10">
          <div className="shrink-0">
            <h1 className="text-lg font-bold tracking-tight">Digitone Presets</h1>
            <p className="text-xs text-slate-500">Your hardware library, organized.</p>
          </div>
          <nav className="topnav">
            {views.map(({ id, label, icon: Icon }) => (
              <button key={id} className={`topnav-item ${view === id ? "topnav-active" : ""}`} onClick={() => setView(id)}>
                <Icon size={16} />{label}
              </button>
            ))}
          </nav>
        </div>
        <div className="flex items-center gap-3">
          <span className={`status-dot ${selectedDevice ? "bg-emerald-400" : "bg-slate-600"}`} />
          <span className="hidden max-w-48 truncate text-sm text-slate-400 lg:block">{selectedDevice?.name ?? "No device"}</span>
          <button className="icon-button" onClick={() => void refreshMidi()} title="Refresh MIDI devices"><RefreshCw size={16} /></button>
        </div>
      </header>

      <main className="mx-auto w-full max-w-[1500px] px-6 py-7">
        {view === "presets" && (
          <section>
            <PageHeading title="Device Presets" subtitle={catalog ? `${deviceTotal} presets synchronized from ${catalog.deviceName}` : "Connect and synchronize your Digitone II to browse its presets."}>
              <button className="primary-button" disabled={!selectedDevice || syncing} onClick={() => void syncDevice()}>
                <RefreshCw size={16} className={syncing ? "animate-spin" : ""} />{catalog ? "Sync again" : "Sync device"}
              </button>
            </PageHeading>
            <div className="mb-5 flex gap-2">{BANKS.map((item) => <button key={item} onClick={() => setBank(item)} className={`bank-tab ${bank === item ? "bank-tab-active" : ""}`}>{item}</button>)}</div>
            <Table headers={["Slot", "Preset", "Local library", "Sound pack(s)", "Tags"]}>
              {(catalog ? deviceRows : localRows).map((preset, index) => {
                const local = localRows[index];
                const name = preset.name;
                return <tr key={preset.slot}><td>{String(preset.slot).padStart(3, "0")}</td><td className="font-medium text-white">{name}</td><td>{local ? <span className="sync-ok"><Check size={13}/>Local</span> : <span className="text-slate-600">Not copied</span>}</td><td>{local ? [...local.exactPacks, ...local.nameOnlyPacks].join(", ") || "—" : "—"}</td><td>{local?.tags.join(", ") || "—"}</td></tr>;
              })}
            </Table>
          </section>
        )}

        {view === "packs" && (
          <section>
            <PageHeading title="Sound Packs" subtitle="Match your device presets with the sound packs in your collection." />
            <label className="mb-5 flex items-center gap-3 text-sm text-slate-400"><input type="checkbox" checked={emptyOnly} onChange={(event) => setEmptyOnly(event.target.checked)} />Show only packs with no matches</label>
            <div className="grid gap-5 xl:grid-cols-[1.1fr_.9fr]">
              <Table headers={["Sound pack", "Found", "Total"]}>{(data?.packs ?? []).filter((pack) => !emptyOnly || pack.found === 0).map((pack) => <tr key={pack.name} onClick={() => setSelectedPack(pack)} className="cursor-pointer hover:bg-white/[.025]"><td className="font-medium text-white">{pack.name}</td><td>{pack.found}</td><td>{pack.total}</td></tr>)}</Table>
              <div className="surface p-6">{selectedPack ? <><h2 className="text-xl font-semibold">{selectedPack.name}</h2><p className="mt-2 text-sm text-slate-500">{selectedPack.found} of {selectedPack.total} found</p><h3 className="section-label">Tags</h3><p className="mt-2 text-sm text-slate-400">{Object.entries(selectedPack.tags).sort((a,b) => b[1]-a[1]).map(([tag,count]) => `${tag} (${count})`).join(", ") || "—"}</p><h3 className="section-label">Device positions</h3>{selectedPack.matches.map((match) => <p key={`${match.location}-${match.name}`} className="mt-2 text-sm text-slate-300">{match.location} · {match.name}</p>)}</> : <p className="text-sm text-slate-500">Select a sound pack to see details.</p>}</div>
            </div>
          </section>
        )}

        {view === "tags" && <section><PageHeading title="Tags" subtitle="Tags found across your synchronized local preset library." /><Table headers={["Tag", "Presets"]}>{tagCounts.map(([tag,count]) => <tr key={tag}><td className="font-medium text-white">{tag}</td><td>{count}</td></tr>)}</Table></section>}

        {view === "settings" && (
          <section>
            <PageHeading title="Settings" subtitle="Configure your device connection and local library." />
            <div className="grid gap-5 xl:grid-cols-2">
              <div className="surface p-6"><h2 className="settings-title"><Usb size={18}/>MIDI device</h2><SettingSelect label="MIDI input" ports={midiInputs} value={selectedInput} onChange={setSelectedInput}/><SettingSelect label="MIDI output" ports={midiOutputs} value={selectedOutput} onChange={setSelectedOutput}/><div className="mt-5 flex gap-3"><button className="secondary-button" onClick={() => void refreshMidi()}><RefreshCw size={16}/>Refresh</button><button className="primary-button" disabled={selectedInput === null || selectedOutput === null || syncing} onClick={() => void syncDevice()}><RefreshCw size={16} className={syncing ? "animate-spin" : ""}/>Synchronize presets</button></div></div>
              <div className="surface p-6"><h2 className="settings-title"><FolderOpen size={18}/>Local library</h2><FolderSetting label="Preset Library" value={settings.banksPath} onClick={() => void chooseFolder("banks")}/><FolderSetting label="Sound Packs" value={settings.packsPath} onClick={() => void chooseFolder("packs")}/><button className="secondary-button mt-5" disabled={!foldersReady} onClick={() => void scanLibrary()}><RefreshCw size={16}/>Rescan library</button></div>
            </div>
          </section>
        )}
      </main>

      <footer className="fixed bottom-0 left-0 right-0 border-t border-white/[.06] bg-[#0b0e12]/95 px-6 py-2 text-xs text-slate-500 backdrop-blur">{message}</footer>
      {(discovering || onboarding) && <Onboarding discovering={discovering} device={selectedDevice} foldersReady={Boolean(settings.banksPath)} syncing={syncing} onClose={() => setOnboarding(false)} onSync={() => void syncDevice()} onSettings={() => { setOnboarding(false); setView("settings"); }} />}
    </div>
  );
}

function PageHeading({ title, subtitle, children }: { title: string; subtitle: string; children?: React.ReactNode }) { return <div className="mb-6 flex items-end justify-between gap-5"><div><h2 className="text-2xl font-semibold tracking-tight">{title}</h2><p className="mt-1 text-sm text-slate-500">{subtitle}</p></div>{children}</div>; }
function Table({ headers, children }: { headers: string[]; children: React.ReactNode }) { return <div className="surface max-h-[calc(100vh-245px)] overflow-auto"><table className="w-full border-collapse"><thead><tr>{headers.map((header) => <th key={header}>{header}</th>)}</tr></thead><tbody>{children}</tbody></table></div>; }
function SettingSelect({ label, ports, value, onChange }: { label: string; ports: MidiPort[]; value: number | null; onChange: (value: number) => void }) { return <label className="mt-5 block text-sm text-slate-400">{label}<select className="field mt-2" value={value ?? ""} onChange={(event) => onChange(Number(event.target.value))}>{ports.length ? ports.map((port) => <option key={port.index} value={port.index}>{port.name}</option>) : <option value="">No MIDI ports found</option>}</select></label>; }
function FolderSetting({ label, value, onClick }: { label: string; value: string | null; onClick: () => void }) { return <div className="mt-5"><span className="text-sm text-slate-400">{label}</span><button className="field mt-2 flex items-center justify-between gap-4 text-left" onClick={onClick}><span className="truncate">{value ?? "Choose folder…"}</span><FolderOpen size={16} className="shrink-0 text-slate-500"/></button></div>; }
function Onboarding({ discovering, device, foldersReady, syncing, onClose, onSync, onSettings }: { discovering: boolean; device?: MidiPort; foldersReady: boolean; syncing: boolean; onClose: () => void; onSync: () => void; onSettings: () => void }) { return <div className="modal-backdrop"><div className="modal-panel">{!discovering && <button className="absolute right-5 top-5 text-slate-600 hover:text-white" onClick={onClose}><X size={18}/></button>}<div className="modal-icon">{discovering ? <LoaderCircle className="animate-spin"/> : <Usb/>}</div>{discovering ? <><h2 className="modal-title">Looking for MIDI devices</h2><p className="modal-copy">Checking the available MIDI input and output ports…</p></> : <><p className="eyebrow">Device detected</p><h2 className="modal-title">{device?.name}</h2><p className="modal-copy">Connect to the device and synchronize its preset banks with your local library.</p><div className="mt-7 space-y-3"><div className="onboarding-step"><Check size={16}/>MIDI input and output are available</div><div className={`onboarding-step ${foldersReady ? "" : "text-amber-300"}`}>{foldersReady ? <Check size={16}/> : <FolderOpen size={16}/>} {foldersReady ? "Local preset library is ready" : "Choose a local preset library first"}</div></div><button className="primary-button mt-7 w-full justify-center py-3" disabled={syncing} onClick={foldersReady ? onSync : onSettings}>{syncing ? <LoaderCircle size={17} className="animate-spin"/> : foldersReady ? <RefreshCw size={17}/> : <FolderOpen size={17}/>} {syncing ? "Synchronizing…" : foldersReady ? "Connect & synchronize" : "Choose library folder"}<ChevronRight size={17}/></button><button className="mt-4 w-full text-center text-sm text-slate-600 hover:text-slate-300" onClick={onClose}>Not now</button></>}</div></div>; }

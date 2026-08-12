import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  Check,
  ChevronRight,
  Filter,
  FolderOpen,
  Library,
  LoaderCircle,
  RefreshCw,
  Search,
  Settings as SettingsIcon,
  Tags,
  Usb,
  X,
} from "lucide-react";
import type { DeviceCatalog, MidiPort, Pack, ScanResult, Settings } from "./types";

const BANKS = [..."ABCDEFGH"];
const PRESETS_PER_BANK = 256;
const views = [
  { id: "presets", label: "Presets", icon: Library },
  { id: "packs", label: "Sound Packs", icon: Archive },
  { id: "tags", label: "Tags", icon: Tags },
  { id: "settings", label: "Settings", icon: SettingsIcon },
] as const;

type View = (typeof views)[number]["id"];
type SyncProgress = { completed: number; total: number; percent: number; bank: string; slot: number; stage: string };

export default function App() {
  const [view, setView] = useState<View>("presets");
  const [bank, setBank] = useState("ALL");
  const [settings, setSettings] = useState<Settings>({ banksPath: null, packsPath: null });
  const [data, setData] = useState<ScanResult | null>(null);
  const [catalog, setCatalog] = useState<DeviceCatalog | null>(null);
  const [midiInputs, setMidiInputs] = useState<MidiPort[]>([]);
  const [midiOutputs, setMidiOutputs] = useState<MidiPort[]>([]);
  const [selectedInput, setSelectedInput] = useState<number | null>(null);
  const [selectedOutput, setSelectedOutput] = useState<number | null>(null);
  const [discovering, setDiscovering] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [syncProgress, setSyncProgress] = useState<SyncProgress | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [onboarding, setOnboarding] = useState(false);
  const [message, setMessage] = useState("Ready");
  const [selectedPack, setSelectedPack] = useState<Pack | null>(null);
  const [emptyOnly, setEmptyOnly] = useState(false);
  const [presetFilter, setPresetFilter] = useState("");
  const [soundPackFilters, setSoundPackFilters] = useState<string[]>([]);
  const [tagFilters, setTagFilters] = useState<string[]>([]);

  const foldersReady = Boolean(settings.banksPath && settings.packsPath);
  const selectedDevice = midiInputs.find((port) => port.index === selectedInput);

  useEffect(() => {
    void initialize();
    const unlisten = listen<SyncProgress>("sync-progress", (event) => setSyncProgress(event.payload));
    return () => { void unlisten.then((dispose) => dispose()); };
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
    setSyncError(null);
    setOnboarding(false);
    setView("presets");
    setMessage("Synchronizing preset catalog…");
    try {
      setSyncProgress({ completed: 0, total: 0, percent: 0, bank: "", slot: 0, stage: "Reading device catalog" });
      const result = await invoke<{ catalog: DeviceCatalog; saved: number }>("sync_device_presets", {
        inputIndex: selectedInput,
        outputIndex: selectedOutput,
        libraryPath: settings.banksPath,
      });
      setCatalog(result.catalog);
      setOnboarding(false);
      setView("presets");
      setMessage(`Synchronization complete · ${result.saved} presets copied`);
      if (settings.packsPath) await scanLibrary();
    } catch (error) {
      const detail = String(error);
      setSyncError(detail);
      setMessage(`Synchronization failed: ${detail}`);
    } finally {
      setSyncing(false);
      setSyncProgress(null);
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

  const visibleBanks = bank === "ALL" ? BANKS : [bank];
  const bankPresetRows = visibleBanks.flatMap((bankName) => {
    const presets = catalog
      ? catalog.banks.find((item) => item.bank === bankName)?.presets ?? []
      : data?.banks[bankName] ?? [];
    return presets.map((preset) => ({ bank: bankName, preset }));
  });
  const soundPackCounts = bankPresetRows.reduce<Record<string, number>>((counts, { bank: rowBank, preset }) => {
    const local = data?.banks[rowBank]?.find((item) => item.slot === preset.slot);
    new Set(local ? [...local.exactPacks, ...local.nameOnlyPacks] : []).forEach((pack) => { counts[pack] = (counts[pack] ?? 0) + 1; });
    return counts;
  }, {});
  const availableSoundPacks = Object.entries(soundPackCounts).sort((a, b) => a[0].localeCompare(b[0]));
  const tagFilterCounts = bankPresetRows.reduce<Record<string, number>>((counts, { bank: rowBank, preset }) => {
    const local = data?.banks[rowBank]?.find((item) => item.slot === preset.slot);
    new Set(local?.tags ?? []).forEach((tag) => { counts[tag] = (counts[tag] ?? 0) + 1; });
    return counts;
  }, {});
  const availableTags = Object.entries(tagFilterCounts).sort((a, b) => a[0].localeCompare(b[0]));
  const presetRows = bankPresetRows.filter(({ bank: rowBank, preset }) => {
    const matchesName = preset.name.toLocaleLowerCase().includes(presetFilter.toLocaleLowerCase());
    const local = data?.banks[rowBank]?.find((item) => item.slot === preset.slot);
    const presetPacks = local ? [...local.exactPacks, ...local.nameOnlyPacks] : [];
    const matchesPack = soundPackFilters.length === 0 || soundPackFilters.some((pack) => presetPacks.includes(pack));
    const matchesTags = tagFilters.length === 0 || Boolean(local?.tags.some((tag) => tagFilters.includes(tag)));
    return matchesName && matchesPack && matchesTags;
  });
  const presetCapacity = visibleBanks.length * PRESETS_PER_BANK;
  const freePresetSlots = Math.max(0, presetCapacity - bankPresetRows.length);
  const filtersActive = Boolean(presetFilter || soundPackFilters.length || tagFilters.length);

  function selectBank(next: string) {
    setBank(next);
    setSoundPackFilters([]);
    setTagFilters([]);
  }

  return (
    <div className="min-h-screen bg-canvas pb-10 text-slate-100">
      <header className="topbar">
        <div className="flex min-w-0 items-center gap-10">
          <div className="shrink-0">
            <h1 className="text-lg font-bold tracking-tight">Digitone2Link</h1>
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
            <div className="mb-5 flex items-center justify-between gap-6"><div className="flex gap-2"><button onClick={() => selectBank("ALL")} className={`bank-tab ${bank === "ALL" ? "bank-tab-active" : ""}`}>ALL</button>{BANKS.map((item) => <button key={item} onClick={() => selectBank(item)} className={`bank-tab ${bank === item ? "bank-tab-active" : ""}`}>{item}</button>)}</div><div className="flex items-center gap-4 whitespace-nowrap text-xs text-slate-500">{filtersActive && <span>Shown: <strong className="ml-1 font-semibold text-emerald-300">{presetRows.length}</strong></span>}<span>Presets: <strong className="ml-1 font-semibold text-slate-300">{bankPresetRows.length}</strong></span><span>Free: <strong className="ml-1 font-semibold text-slate-300">{freePresetSlots}</strong></span></div></div>
            <Table headers={bank === "ALL" ? ["Bank", "Slot", <PresetSearchHeader value={presetFilter} onFilter={setPresetFilter}/>, "Sync", <SoundPackFilterHeader options={availableSoundPacks} values={soundPackFilters} onFilter={setSoundPackFilters}/>, <TagsFilterHeader options={availableTags} values={tagFilters} onFilter={setTagFilters}/>] : ["Slot", <PresetSearchHeader value={presetFilter} onFilter={setPresetFilter}/>, "Sync", <SoundPackFilterHeader options={availableSoundPacks} values={soundPackFilters} onFilter={setSoundPackFilters}/>, <TagsFilterHeader options={availableTags} values={tagFilters} onFilter={setTagFilters}/>]}>
              {presetRows.map(({ bank: rowBank, preset }) => {
                const local = data?.banks[rowBank]?.find((item) => item.slot === preset.slot);
                const name = preset.name;
                return <tr key={`${rowBank}-${preset.slot}`}>{bank === "ALL" && <td className="font-semibold text-emerald-300">{rowBank}</td>}<td>{String(preset.slot).padStart(3, "0")}</td><td className="font-medium text-white">{name}</td><td>{local ? <span className="sync-ok h-7 w-7 justify-center p-0" title="Synchronized"><Check size={14}/></span> : <span className="text-slate-700" title="Not synchronized">—</span>}</td><td>{local ? [...local.exactPacks, ...local.nameOnlyPacks].join(", ") || "—" : "—"}</td><td>{local?.tags.length ? <div className="preset-tags">{Array.from(new Set(local.tags)).map((tag) => <span key={tag} className="preset-tag">{tag}</span>)}</div> : "—"}</td></tr>;
              })}
            </Table>
          </section>
        )}

        {view === "packs" && (
          <section>
            <label className="mb-5 flex items-center gap-3 text-sm text-slate-400"><input type="checkbox" checked={emptyOnly} onChange={(event) => setEmptyOnly(event.target.checked)} />Show only packs with no matches</label>
            <div className="grid gap-5 xl:grid-cols-[1.1fr_.9fr]">
              <Table headers={["Sound pack", "Found", "Total"]}>{(data?.packs ?? []).filter((pack) => !emptyOnly || pack.found === 0).map((pack) => <tr key={pack.name} onClick={() => setSelectedPack(pack)} className="cursor-pointer hover:bg-white/[.025]"><td className="font-medium text-white">{pack.name}</td><td>{pack.found}</td><td>{pack.total}</td></tr>)}</Table>
              <div className="surface p-6">{selectedPack ? <><h2 className="text-xl font-semibold">{selectedPack.name}</h2><p className="mt-2 text-sm text-slate-500">{selectedPack.found} of {selectedPack.total} found</p><h3 className="section-label">Tags</h3><p className="mt-2 text-sm text-slate-400">{Object.entries(selectedPack.tags).sort((a,b) => b[1]-a[1]).map(([tag,count]) => `${tag} (${count})`).join(", ") || "—"}</p><h3 className="section-label">Device positions</h3>{selectedPack.matches.map((match) => <p key={`${match.location}-${match.name}`} className="mt-2 text-sm text-slate-300">{match.location} · {match.name}</p>)}</> : <p className="text-sm text-slate-500">Select a sound pack to see details.</p>}</div>
            </div>
          </section>
        )}

        {view === "tags" && <section><Table headers={["Tag", "Presets"]}>{tagCounts.map(([tag,count]) => <tr key={tag}><td className="font-medium text-white">{tag}</td><td>{count}</td></tr>)}</Table></section>}

        {view === "settings" && (
          <section>
            <div className="grid gap-5 xl:grid-cols-2">
              <div className="surface p-6"><h2 className="settings-title"><Usb size={18}/>MIDI device</h2><SettingSelect label="MIDI input" ports={midiInputs} value={selectedInput} onChange={setSelectedInput}/><SettingSelect label="MIDI output" ports={midiOutputs} value={selectedOutput} onChange={setSelectedOutput}/><div className="mt-5 flex gap-3"><button className="secondary-button" onClick={() => void refreshMidi()}><RefreshCw size={16}/>Refresh</button><button className="primary-button" disabled={selectedInput === null || selectedOutput === null || syncing} onClick={() => void syncDevice()}><RefreshCw size={16} className={syncing ? "animate-spin" : ""}/>Synchronize presets</button></div></div>
              <div className="surface p-6"><h2 className="settings-title"><FolderOpen size={18}/>Local library</h2><FolderSetting label="Preset Library" value={settings.banksPath} onClick={() => void chooseFolder("banks")}/><FolderSetting label="Sound Packs" value={settings.packsPath} onClick={() => void chooseFolder("packs")}/><button className="secondary-button mt-5" disabled={!foldersReady} onClick={() => void scanLibrary()}><RefreshCw size={16}/>Rescan library</button></div>
            </div>
          </section>
        )}
      </main>

      <footer className="fixed bottom-0 left-0 right-0 z-30 border-t border-white/[.06] bg-[#0b0e12]/95 backdrop-blur">
        {syncing && syncProgress ? <BackgroundSyncStatus progress={syncProgress} /> : <div className="px-6 py-2 text-xs text-slate-500">{message}</div>}
      </footer>
      {(discovering || onboarding) && <Onboarding discovering={discovering} inputs={midiInputs} outputs={midiOutputs} selectedInput={selectedInput} selectedOutput={selectedOutput} onInput={setSelectedInput} onOutput={setSelectedOutput} foldersReady={Boolean(settings.banksPath)} syncing={syncing} onRefresh={() => void refreshMidi()} onClose={() => setOnboarding(false)} onSync={() => void syncDevice()} onSettings={() => { setOnboarding(false); setView("settings"); }} />}
      {!syncing && syncError && <SyncError detail={syncError} onClose={() => setSyncError(null)} onRetry={() => { setSyncError(null); void syncDevice(); }} />}
    </div>
  );
}

function Table({ headers, children }: { headers: React.ReactNode[]; children: React.ReactNode }) { return <div className="surface max-h-[calc(100vh-245px)] overflow-auto"><table className="w-full border-collapse"><thead><tr>{headers.map((header, index) => <th key={index}>{header}</th>)}</tr></thead><tbody>{children}</tbody></table></div>; }
function PresetSearchHeader({ value, onFilter }: { value: string; onFilter: (value: string) => void }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  function update(next: string) {
    setText(next);
    const query = next.trim();
    if (query.length >= 3 || query.length === 0) onFilter(query);
  }

  function close() {
    setOpen(false);
    setText("");
    onFilter("");
  }

  return <div><div className="inline-flex items-center gap-1.5">Preset<button className={`header-search-button ${value ? "text-emerald-300" : ""}`} title="Search presets" onClick={() => setOpen((current) => !current)}><Search size={13}/></button></div>{open && <div className="header-search-popover"><Search size={15} className="shrink-0 text-slate-500"/><input ref={inputRef} value={text} onChange={(event) => update(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") onFilter(text.trim()); if (event.key === "Escape") close(); }} placeholder="Preset name…"/><button className="text-slate-600 transition hover:text-white" title="Clear and close" onClick={close}><X size={14}/></button></div>}</div>;
}
function SoundPackFilterHeader({ options, values, onFilter }: { options: [string, number][]; values: string[]; onFilter: (values: string[]) => void }) {
  const [open, setOpen] = useState(false);
  function toggle(pack: string) {
    onFilter(values.includes(pack) ? values.filter((value) => value !== pack) : [...values, pack]);
  }
  return <div><div className="inline-flex items-center gap-1.5">Sound pack(s)<button className={`header-search-button ${values.length ? "text-emerald-300" : ""}`} title="Filter by sound packs" onClick={() => setOpen((current) => !current)}><Filter size={13}/></button></div>{open && <div className="header-tags-panel"><div className="flex items-center justify-between border-b border-white/[.07] px-3 py-2"><span className="text-xs font-medium normal-case tracking-normal text-slate-400">Match any pack{values.length ? ` · ${values.length} selected` : ""}</span><div className="flex items-center gap-2">{values.length > 0 && <button className="text-xs font-medium normal-case tracking-normal text-emerald-400 hover:text-emerald-300" onClick={() => onFilter([])}>Clear</button>}<button className="text-slate-600 transition hover:text-white" title="Close" onClick={() => setOpen(false)}><X size={14}/></button></div></div><div className="tag-filter-cloud">{options.length ? options.map(([pack, count]) => <label key={pack} className={`tag-filter-option ${values.includes(pack) ? "tag-filter-option-active" : ""}`}><input type="checkbox" checked={values.includes(pack)} onChange={() => toggle(pack)}/><span>{pack} — {count}</span></label>) : <p className="px-2 py-3 text-xs font-normal normal-case tracking-normal text-slate-600">No sound packs available</p>}</div></div>}</div>;
}
function TagsFilterHeader({ options, values, onFilter }: { options: [string, number][]; values: string[]; onFilter: (values: string[]) => void }) {
  const [open, setOpen] = useState(false);
  function toggle(tag: string) {
    onFilter(values.includes(tag) ? values.filter((value) => value !== tag) : [...values, tag]);
  }
  return <div><div className="inline-flex items-center gap-1.5">Tags<button className={`header-search-button ${values.length ? "text-emerald-300" : ""}`} title="Filter by tags" onClick={() => setOpen((current) => !current)}><Filter size={13}/></button></div>{open && <div className="header-tags-panel"><div className="flex items-center justify-between border-b border-white/[.07] px-3 py-2"><span className="text-xs font-medium normal-case tracking-normal text-slate-400">Match any tag{values.length ? ` · ${values.length} selected` : ""}</span><div className="flex items-center gap-2">{values.length > 0 && <button className="text-xs font-medium normal-case tracking-normal text-emerald-400 hover:text-emerald-300" onClick={() => onFilter([])}>Clear</button>}<button className="text-slate-600 transition hover:text-white" title="Close" onClick={() => setOpen(false)}><X size={14}/></button></div></div><div className="tag-filter-cloud">{options.length ? options.map(([tag, count]) => <label key={tag} className={`tag-filter-option ${values.includes(tag) ? "tag-filter-option-active" : ""}`}><input type="checkbox" checked={values.includes(tag)} onChange={() => toggle(tag)}/><span>{tag} — {count}</span></label>) : <p className="px-2 py-3 text-xs font-normal normal-case tracking-normal text-slate-600">No tags available</p>}</div></div>}</div>;
}
function SettingSelect({ label, ports, value, onChange }: { label: string; ports: MidiPort[]; value: number | null; onChange: (value: number) => void }) { return <label className="mt-5 block text-sm text-slate-400">{label}<select className="field mt-2" value={value ?? ""} onChange={(event) => onChange(Number(event.target.value))}>{ports.length ? ports.map((port) => <option key={port.index} value={port.index}>{port.name}</option>) : <option value="">No MIDI ports found</option>}</select></label>; }
function FolderSetting({ label, value, onClick }: { label: string; value: string | null; onClick: () => void }) { return <div className="mt-5"><span className="text-sm text-slate-400">{label}</span><button className="field mt-2 flex items-center justify-between gap-4 text-left" onClick={onClick}><span className="truncate">{value ?? "Choose folder…"}</span><FolderOpen size={16} className="shrink-0 text-slate-500"/></button></div>; }
function Onboarding({ discovering, inputs, outputs, selectedInput, selectedOutput, onInput, onOutput, foldersReady, syncing, onRefresh, onClose, onSync, onSettings }: { discovering: boolean; inputs: MidiPort[]; outputs: MidiPort[]; selectedInput: number | null; selectedOutput: number | null; onInput: (value: number) => void; onOutput: (value: number) => void; foldersReady: boolean; syncing: boolean; onRefresh: () => void; onClose: () => void; onSync: () => void; onSettings: () => void }) {
  const connectionReady = selectedInput !== null && selectedOutput !== null;
  return <div className="modal-backdrop"><div className="modal-panel">{!discovering && <button className="absolute right-5 top-5 text-slate-600 hover:text-white" onClick={onClose}><X size={18}/></button>}<div className="modal-icon">{discovering ? <LoaderCircle className="animate-spin"/> : <Usb/>}</div>{discovering ? <><h2 className="modal-title">Looking for MIDI devices</h2><p className="modal-copy">Checking the available MIDI input and output ports…</p></> : <><p className="eyebrow">MIDI connection</p><h2 className="modal-title">Select your Digitone</h2><p className="modal-copy">Check both MIDI ports before synchronizing the preset library.</p><div className="mt-6 space-y-4 text-left"><SettingSelect label="MIDI input" ports={inputs} value={selectedInput} onChange={onInput}/><SettingSelect label="MIDI output" ports={outputs} value={selectedOutput} onChange={onOutput}/><button className="flex items-center gap-2 text-xs text-slate-500 hover:text-slate-300" onClick={onRefresh}><RefreshCw size={13}/>Refresh MIDI devices</button></div><div className="mt-6 space-y-3"><div className={`onboarding-step ${connectionReady ? "" : "text-amber-300"}`}>{connectionReady ? <Check size={16}/> : <Usb size={16}/>} {connectionReady ? "MIDI input and output are selected" : "Select both MIDI ports"}</div><div className={`onboarding-step ${foldersReady ? "" : "text-amber-300"}`}>{foldersReady ? <Check size={16}/> : <FolderOpen size={16}/>} {foldersReady ? "Local preset library is ready" : "Choose a local preset library first"}</div></div><button className="primary-button mt-7 w-full justify-center py-3" disabled={syncing || !connectionReady} onClick={foldersReady ? onSync : onSettings}>{syncing ? <LoaderCircle size={17} className="animate-spin"/> : foldersReady ? <RefreshCw size={17}/> : <FolderOpen size={17}/>} {syncing ? "Synchronizing…" : foldersReady ? "Connect & synchronize" : "Choose library folder"}<ChevronRight size={17}/></button><button className="mt-4 w-full text-center text-sm text-slate-600 hover:text-slate-300" onClick={onClose}>Not now</button></>}</div></div>;
}
function BackgroundSyncStatus({ progress }: { progress: SyncProgress }) {
  const location = progress.bank ? progress.slot ? `Bank ${progress.bank} · slot ${String(progress.slot).padStart(3, "0")}` : `Bank ${progress.bank}` : progress.stage;
  return <div className="grid items-center gap-3 px-6 py-2.5 md:grid-cols-[minmax(220px,1fr)_minmax(240px,2fr)_auto]"><div className="flex min-w-0 items-center gap-2 text-xs"><RefreshCw size={14} className="shrink-0 animate-spin text-emerald-400"/><span className="font-medium text-slate-300">Synchronizing device</span><span className="truncate text-slate-600">{location}</span></div><div className="flex items-center gap-3"><div className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/[.08]"><div className="h-full rounded-full bg-emerald-400 transition-[width] duration-300" style={{ width: `${progress.percent}%` }}/></div><span className="w-9 text-right text-xs font-semibold text-emerald-300">{progress.percent}%</span></div><div className="whitespace-nowrap text-right text-xs text-slate-500">{progress.total ? `${progress.completed} / ${progress.total} ${progress.slot ? "presets" : "banks"}` : "Reading catalog…"}</div></div>;
}
function SyncError({ detail, onClose, onRetry }: { detail: string; onClose: () => void; onRetry: () => void }) { return <div className="modal-backdrop"><div className="modal-panel"><button className="absolute right-5 top-5 text-slate-600 hover:text-white" onClick={onClose}><X size={18}/></button><div className="modal-icon border-red-400/20 bg-red-400/10 text-red-300"><X/></div><p className="eyebrow text-red-300">Synchronization failed</p><h2 className="modal-title">The device could not be read</h2><p className="modal-copy">No local preset files were changed.</p><pre className="mt-6 max-h-40 overflow-auto whitespace-pre-wrap rounded-lg border border-white/[.08] bg-black/30 p-4 text-left text-xs text-red-200">{detail}</pre><div className="mt-6 flex gap-3"><button className="secondary-button flex-1 justify-center" onClick={onClose}>Close</button><button className="primary-button flex-1 justify-center" onClick={onRetry}><RefreshCw size={16}/>Retry</button></div></div></div>; }

import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  ArrowUpDown,
  Check,
  ChevronRight,
  Copy,
  Filter,
  FolderOpen,
  Library,
  Lock,
  LoaderCircle,
  Pause,
  Play,
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
  const [syncPaused, setSyncPaused] = useState(false);
  const [syncProgress, setSyncProgress] = useState<SyncProgress | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [onboarding, setOnboarding] = useState(false);
  const [message, setMessage] = useState("Ready");
  const [selectedPack, setSelectedPack] = useState<Pack | null>(null);
  const [packSearch, setPackSearch] = useState("");
  const [packTagFilters, setPackTagFilters] = useState<string[]>([]);
  const [packSort, setPackSort] = useState<"name" | "used" | "total">("used");
  const [presetFilter, setPresetFilter] = useState("");
  const [soundPackFilters, setSoundPackFilters] = useState<string[]>([]);
  const [tagFilters, setTagFilters] = useState<string[]>([]);
  const [duplicatesOnly, setDuplicatesOnly] = useState(false);

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

  async function syncDevice(targetBank?: string) {
    if (selectedInput === null || selectedOutput === null) return;
    if (!settings.banksPath) {
      setOnboarding(false);
      setView("settings");
      setMessage("Choose a local Preset Library folder before synchronization");
      return;
    }
    setSyncing(true);
    setSyncPaused(false);
    setSyncError(null);
    setOnboarding(false);
    setView("presets");
    setMessage(targetBank ? `Synchronizing bank ${targetBank}…` : "Synchronizing preset catalog…");
    try {
      setSyncProgress({ completed: 0, total: 0, percent: 0, bank: "", slot: 0, stage: "Reading device catalog" });
      const result = await invoke<{ catalog: DeviceCatalog; saved: number }>("sync_device_presets", {
        inputIndex: selectedInput,
        outputIndex: selectedOutput,
        libraryPath: settings.banksPath,
        bank: targetBank ?? null,
      });
      setCatalog((current) => {
        if (!targetBank || !current) return result.catalog;
        const replacement = result.catalog.banks[0];
        if (!replacement) return current;
        return { ...current, deviceName: result.catalog.deviceName, banks: current.banks.map((item) => item.bank === targetBank ? replacement : item) };
      });
      setOnboarding(false);
      setView("presets");
      setMessage(`${targetBank ? `Bank ${targetBank}` : "Synchronization"} complete · ${result.saved} presets copied`);
      if (settings.packsPath) await scanLibrary();
    } catch (error) {
      const detail = String(error);
      setSyncError(detail);
      setMessage(`Synchronization failed: ${detail}`);
    } finally {
      setSyncing(false);
      setSyncPaused(false);
      setSyncProgress(null);
    }
  }

  async function toggleSyncPause() {
    if (!syncing) return;
    const paused = !syncPaused;
    await invoke("set_preset_sync_paused", { paused });
    setSyncPaused(paused);
    setMessage(paused ? "Synchronization paused" : "Synchronization resumed");
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
  const largestTagCount = tagCounts[0]?.[1] ?? 0;
  const smallestTagCount = tagCounts[tagCounts.length - 1]?.[1] ?? 0;
  const tagCloudRows = useMemo(() => buildTagCloudRows(tagCounts), [tagCounts]);

  const duplicatePresets = Object.values(data?.banks ?? {}).flat().filter((preset) => preset.duplicateLocations.length > 0);
  const duplicateGroupKeys = new Set(duplicatePresets.map((preset) => [
    `${preset.bank}${String(preset.slot).padStart(3, "0")}`,
    ...preset.duplicateLocations,
  ].sort().join("|")));
  const duplicateExtraCount = Math.max(0, duplicatePresets.length - duplicateGroupKeys.size);
  const showAllBanks = bank === "ALL" || duplicatesOnly;
  const visibleBanks = showAllBanks ? BANKS : [bank];
  const bankPresetCounts = Object.fromEntries(BANKS.map((bankName) => [
    bankName,
    catalog
      ? catalog.banks.find((item) => item.bank === bankName)?.presets.length ?? 0
      : data?.banks[bankName]?.length ?? 0,
  ]));
  const allBankPresetCount = Object.values(bankPresetCounts).reduce((total, count) => total + count, 0);
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
    const matchesDuplicates = !duplicatesOnly || Boolean(local?.duplicateLocations.length);
    return matchesName && matchesPack && matchesTags && matchesDuplicates;
  });
  const presetCapacity = visibleBanks.length * PRESETS_PER_BANK;
  const freePresetSlots = Math.max(0, presetCapacity - bankPresetRows.length);
  const filtersActive = Boolean(duplicatesOnly || presetFilter || soundPackFilters.length || tagFilters.length);
  const packTagOptions = useMemo(() => {
    const counts: Record<string, number> = {};
    (data?.packs ?? []).forEach((pack) => Object.keys(pack.tags).forEach((tag) => { counts[tag] = (counts[tag] ?? 0) + 1; }));
    return Object.entries(counts).sort((a, b) => a[0].localeCompare(b[0]));
  }, [data]);
  const filteredPacks = useMemo(() => {
    const query = packSearch.toLocaleLowerCase();
    return (data?.packs ?? [])
      .filter((pack) => pack.name.toLocaleLowerCase().includes(query)
        && (packTagFilters.length === 0 || pack.presets.some((preset) => packTagFilters.some((tag) => preset.tags.includes(tag)))))
      .sort((a, b) => {
        const aScope = packTagFilters.length ? a.presets.filter((preset) => packTagFilters.some((tag) => preset.tags.includes(tag))) : a.presets;
        const bScope = packTagFilters.length ? b.presets.filter((preset) => packTagFilters.some((tag) => preset.tags.includes(tag))) : b.presets;
        const aUsed = aScope.filter((preset) => preset.used).length;
        const bUsed = bScope.filter((preset) => preset.used).length;
        if (packSort === "used") return bUsed - aUsed || (bUsed / Math.max(bScope.length, 1)) - (aUsed / Math.max(aScope.length, 1)) || a.name.localeCompare(b.name);
        if (packSort === "total") return bScope.length - aScope.length || a.name.localeCompare(b.name);
        return a.name.localeCompare(b.name);
      });
  }, [data, packSearch, packTagFilters, packSort]);
  const largestUsedPresetCount = Math.max(0, ...filteredPacks.map((pack) => (packTagFilters.length ? pack.presets.filter((preset) => packTagFilters.some((tag) => preset.tags.includes(tag))) : pack.presets).filter((preset) => preset.used).length));
  const activePack = selectedPack ? filteredPacks.find((pack) => pack.name === selectedPack.name) ?? null : null;
  const packSummary = useMemo(() => {
    const packs = data?.packs ?? [];
    const libraryPresets = Object.values(data?.banks ?? {}).flat();
    const matchedLibraryPresets = libraryPresets.filter((preset) => preset.exactPacks.length > 0 || preset.nameOnlyPacks.length > 0).length;
    const usedPacks = packs.filter((pack) => pack.found > 0).length;
    return { total: packs.length, used: usedPacks, unused: packs.length - usedPacks, coverage: libraryPresets.length ? Math.round(matchedLibraryPresets * 100 / libraryPresets.length) : 0 };
  }, [data]);

  function selectBank(next: string) {
    setBank(next);
    setDuplicatesOnly(false);
    setSoundPackFilters([]);
    setTagFilters([]);
  }

  function toggleDuplicates() {
    setDuplicatesOnly((current) => !current);
    setPresetFilter("");
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
            <div className="mb-5 flex items-center justify-between gap-6"><div className="flex gap-2"><BankTab label="ALL" count={allBankPresetCount} capacity={BANKS.length * PRESETS_PER_BANK} active={bank === "ALL" && !duplicatesOnly} onClick={() => selectBank("ALL")}/>{BANKS.map((item) => <BankTab key={item} label={item} count={bankPresetCounts[item]} capacity={PRESETS_PER_BANK} active={bank === item && !duplicatesOnly} onClick={() => selectBank(item)}/>)}<span className="mx-1 border-l border-white/[.08]"/><button className={`duplicates-tab ${duplicatesOnly ? "duplicates-tab-active" : ""}`} disabled={duplicateExtraCount === 0} onClick={toggleDuplicates} title={`${duplicateGroupKeys.size} duplicate groups · ${duplicatePresets.length} matching presets`}><Copy size={12}/>Duplicates <strong>{duplicateExtraCount}</strong></button></div><div className="flex items-center gap-4 whitespace-nowrap text-xs text-slate-500">{filtersActive && <span>Shown: <strong className="ml-1 font-semibold text-emerald-300">{presetRows.length}</strong></span>}<span>Presets: <strong className="ml-1 font-semibold text-slate-300">{bankPresetRows.length}</strong></span><span>Free: <strong className="ml-1 font-semibold text-slate-300">{freePresetSlots}</strong></span>{bank !== "ALL" && !duplicatesOnly && <button className="bank-sync-button" disabled={syncing || selectedInput === null || selectedOutput === null || !settings.banksPath} onClick={() => void syncDevice(bank)} title={`Synchronize bank ${bank}`}><RefreshCw size={13} className={syncing ? "animate-spin" : ""}/>Sync bank {bank}</button>}</div></div>
            <Table fillViewport headers={showAllBanks ? ["Bank", "Slot", <PresetSearchHeader value={presetFilter} onFilter={setPresetFilter}/>, "Sync", <SoundPackFilterHeader options={availableSoundPacks} values={soundPackFilters} onFilter={setSoundPackFilters}/>, <TagsFilterHeader options={availableTags} values={tagFilters} onFilter={setTagFilters}/>] : ["Slot", <PresetSearchHeader value={presetFilter} onFilter={setPresetFilter}/>, "Sync", <SoundPackFilterHeader options={availableSoundPacks} values={soundPackFilters} onFilter={setSoundPackFilters}/>, <TagsFilterHeader options={availableTags} values={tagFilters} onFilter={setTagFilters}/>]}>
              {presetRows.map(({ bank: rowBank, preset }) => {
                const local = data?.banks[rowBank]?.find((item) => item.slot === preset.slot);
                const name = preset.name;
                return <tr key={`${rowBank}-${preset.slot}`}>{showAllBanks && <td className="font-semibold text-emerald-300">{rowBank}</td>}<td>{String(preset.slot).padStart(3, "0")}</td><td className="font-medium text-white">{name}</td><td>{local ? <span className="sync-ok h-7 w-7 justify-center p-0" title="Synchronized"><Check size={14}/></span> : <span className="text-slate-700" title="Not synchronized">—</span>}</td><td>{local ? [...local.exactPacks, ...local.nameOnlyPacks].join(", ") || "—" : "—"}</td><td>{local?.tags.length ? <div className="preset-tags">{Array.from(new Set(local.tags)).map((tag) => <span key={tag} className="preset-tag">{tag}</span>)}</div> : "—"}</td></tr>;
              })}
            </Table>
          </section>
        )}

        {view === "packs" && (
          <section>
            <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
              <div className="pack-summary"><span>Sound packs <strong>{packSummary.total}</strong></span><i/><span>Used <strong>{packSummary.used}</strong></span><span>Unused <strong>{packSummary.unused}</strong></span><i/><span>Library coverage <strong className="text-emerald-300">{packSummary.coverage}%</strong></span></div>
              <div className="flex items-center gap-3"><TagsFilterHeader options={packTagOptions} values={packTagFilters} onFilter={setPackTagFilters}/><label className="flex items-center gap-2 text-xs text-slate-500"><ArrowUpDown size={14}/><span>Sort</span><select className="pack-sort" value={packSort} onChange={(event) => setPackSort(event.target.value as typeof packSort)}><option value="name">Name A–Z</option><option value="used">Used presets</option><option value="total">Preset count</option></select></label></div>
            </div>
            <div className="grid items-start gap-5 xl:grid-cols-[minmax(520px,.92fr)_minmax(580px,1.08fr)]">
              <Table className="pack-list-table" headers={[<PackSearchHeader value={packSearch} onFilter={setPackSearch}/>, "Presets", "Used"]}>{filteredPacks.map((pack) => { const scopedPresets = packTagFilters.length ? pack.presets.filter((preset) => packTagFilters.some((tag) => preset.tags.includes(tag))) : pack.presets; const scopedUsed = scopedPresets.filter((preset) => preset.used).length; const percent = scopedPresets.length ? Math.round(scopedUsed * 100 / scopedPresets.length) : 0; const relativeUsage = largestUsedPresetCount ? scopedUsed * 100 / largestUsedPresetCount : 0; return <tr key={pack.name} onClick={() => setSelectedPack(pack)} className={`pack-row cursor-pointer ${activePack?.name === pack.name ? "pack-row-active" : ""}`}><td className="relative overflow-hidden font-medium text-white"><span className="pack-usage-fill" style={{ width: `${relativeUsage}%` }}/><span className="relative z-[1] flex min-w-0 items-center gap-1.5"><span className="truncate" title={pack.name}>{pack.name}</span>{pack.name === "Factory" && <Lock size={12} className="shrink-0 text-slate-500" aria-label="Built-in factory pack"/>}</span></td><td>{scopedPresets.length}</td><td><span className="font-semibold text-emerald-300">{scopedUsed}</span><span className="ml-1 text-slate-600">{percent}%</span></td></tr>; })}</Table>
              <PackDetails pack={activePack} filterTags={packTagFilters}/>
            </div>
          </section>
        )}

        {view === "tags" && <section>{tagCounts.length ? <div className="tag-cloud">{tagCloudRows.map((row, rowIndex) => <div key={rowIndex} className="tag-cloud-row" style={{ width: `${row.width}%` }}>{row.tags.map(([tag, count]) => { const range = largestTagCount - smallestTagCount; const weight = range ? Math.sqrt((count - smallestTagCount) / range) : 0.5; return <span key={tag} className="tag-cloud-item" style={{ fontSize: `${11 + weight * 13}px`, padding: `${5 + weight * 4}px ${9 + weight * 6}px`, opacity: 0.65 + weight * 0.35, borderColor: `rgba(52, 211, 153, ${0.12 + weight * 0.28})`, backgroundColor: `rgba(52, 211, 153, ${0.025 + weight * 0.09})` }}><span>{tag}</span><small>{count}</small></span>; })}</div>)}</div> : <div className="surface p-10 text-center text-sm text-slate-500">No tags found. Synchronize and scan your preset library first.</div>}</section>}

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
        <div className="flex min-h-9 items-stretch">
          <div className="status-message min-w-0 flex-1"><span>{message}</span></div>
          <div className="flex shrink-0 items-center border-l border-white/[.06]">
            {syncing && syncProgress ? <BackgroundSyncStatus progress={syncProgress} paused={syncPaused} onTogglePause={() => void toggleSyncPause()} /> : <div className="px-4 text-[10px] font-medium uppercase tracking-wider text-slate-700">Sync idle</div>}
          </div>
        </div>
      </footer>
      {(discovering || onboarding) && <Onboarding discovering={discovering} inputs={midiInputs} outputs={midiOutputs} selectedInput={selectedInput} selectedOutput={selectedOutput} onInput={setSelectedInput} onOutput={setSelectedOutput} foldersReady={Boolean(settings.banksPath)} syncing={syncing} onRefresh={() => void refreshMidi()} onClose={() => setOnboarding(false)} onSync={() => void syncDevice()} onSettings={() => { setOnboarding(false); setView("settings"); }} />}
      {!syncing && syncError && <SyncError detail={syncError} onClose={() => setSyncError(null)} onRetry={() => { setSyncError(null); void syncDevice(); }} />}
    </div>
  );
}

function BankTab({ label, count, capacity, active, onClick }: { label: string; count: number; capacity: number; active: boolean; onClick: () => void }) {
  const percent = capacity ? Math.min(100, Math.round(count * 100 / capacity)) : 0;
  const bankLabel = label === "ALL" ? "All banks" : `Bank ${label}`;
  return <button onClick={onClick} className={`bank-tab ${active ? "bank-tab-active" : ""}`} title={`${bankLabel}: ${count} / ${capacity} presets · ${percent}% full`}><span className="bank-tab-label">{label}</span><span className={`bank-tab-fill ${percent === 100 ? "bank-tab-fill-full" : ""}`} style={{ width: `${percent}%` }}/></button>;
}

function Table({ headers, children, className = "", fillViewport = false }: { headers: React.ReactNode[]; children: React.ReactNode; className?: string; fillViewport?: boolean }) { return <div className={`surface max-h-[calc(100vh-210px)] overflow-auto ${fillViewport ? "min-h-[calc(100vh-210px)]" : ""}`}><table className={`w-full border-collapse ${className}`}><thead><tr>{headers.map((header, index) => <th key={index}>{header}</th>)}</tr></thead><tbody>{children}</tbody></table></div>; }
function buildTagCloudRows(tags: [string, number][]) {
  const rowCount = tags.length > 48 ? 9 : 7;
  const middle = (rowCount - 1) / 2;
  const widths = Array.from({ length: rowCount }, (_, index) => {
    const position = (index - middle) / (middle + 0.7);
    return Math.round(Math.sqrt(1 - position * position) * 94);
  });
  const totalWidth = widths.reduce((sum, width) => sum + width, 0);
  const counts = widths.map((width) => Math.floor(tags.length * width / totalWidth));
  for (let remaining = tags.length - counts.reduce((sum, count) => sum + count, 0); remaining > 0; remaining -= 1) {
    const index = Array.from({ length: rowCount }, (_, row) => row).sort((a, b) => Math.abs(a - middle) - Math.abs(b - middle))[remaining % rowCount];
    counts[index] += 1;
  }
  const rows = counts.map(() => [] as [string, number][]);
  const fillOrder = Array.from({ length: rowCount }, (_, index) => index).sort((a, b) => Math.abs(a - middle) - Math.abs(b - middle));
  let cursor = 0;
  fillOrder.forEach((row) => {
    rows[row] = tags.slice(cursor, cursor + counts[row]);
    cursor += counts[row];
  });
  return rows.map((row, index) => ({ tags: row, width: widths[index] }));
}
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
function PackSearchHeader({ value, onFilter }: { value: string; onFilter: (value: string) => void }) {
  const [open, setOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => { if (open) inputRef.current?.focus(); }, [open]);
  return <div><div className="inline-flex items-center gap-1.5">Sound pack<button className={`header-search-button ${value ? "text-emerald-300" : ""}`} title="Search sound packs" onClick={() => setOpen((current) => !current)}><Search size={13}/></button></div>{open && <div className="header-search-popover"><Search size={15} className="shrink-0 text-slate-500"/><input ref={inputRef} value={value} onChange={(event) => onFilter(event.target.value)} onKeyDown={(event) => { if (event.key === "Escape") { onFilter(""); setOpen(false); } }} placeholder="Sound pack name…"/><button className="text-slate-600 transition hover:text-white" title="Clear and close" onClick={() => { onFilter(""); setOpen(false); }}><X size={14}/></button></div>}</div>;
}
function PackDetails({ pack, filterTags }: { pack: Pack | null; filterTags: string[] }) {
  const [usedOnly, setUsedOnly] = useState(false);
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  useEffect(() => { setSelectedTags([]); setUsedOnly(false); }, [pack?.name, filterTags.join("|")]);
  if (!pack) return <div className="surface flex min-h-[360px] items-center justify-center p-10 text-sm text-slate-500">Select a sound pack to see its presets.</div>;
  const scopedPresets = filterTags.length ? pack.presets.filter((preset) => filterTags.some((tag) => preset.tags.includes(tag))) : pack.presets;
  const scopedFound = scopedPresets.filter((preset) => preset.used).length;
  const percent = scopedPresets.length ? Math.round(scopedFound * 100 / scopedPresets.length) : 0;
  const scopedTags = scopedPresets.reduce<Record<string, number>>((counts, preset) => { preset.tags.forEach((tag) => { counts[tag] = (counts[tag] ?? 0) + 1; }); return counts; }, {});
  const presets = scopedPresets.filter((preset) => (!usedOnly || preset.used) && (selectedTags.length === 0 || selectedTags.some((tag) => preset.tags.includes(tag))));
  function toggleTag(tag: string) { setSelectedTags((current) => current.includes(tag) ? current.filter((value) => value !== tag) : [...current, tag]); }
  return <div className="surface flex max-h-[calc(100vh-245px)] min-h-[560px] flex-col overflow-hidden">
    <div className="border-b border-white/[.07] p-6">
      <div className="flex items-start justify-between gap-5"><div className="min-w-0 flex-1"><h2 className="pack-title flex min-w-0 items-start gap-2 text-xl font-semibold text-white"><span className="min-w-0">{pack.name}</span>{pack.name === "Factory" && <Lock size={15} className="mt-1 shrink-0 text-slate-500" aria-label="Built-in factory pack"/>}</h2><div className="pack-progress mt-4"><div className="pack-progress-fill" style={{ width: `${percent}%` }}/><div className="pack-progress-label"><span><strong>{scopedFound}</strong> of {scopedPresets.length} presets used</span><strong>{percent}%</strong></div></div></div>{pack.coverDataUrl && <img className="pack-cover" src={pack.coverDataUrl} alt={`${pack.name} cover`}/>}</div>
      <div className="mt-5 flex items-center gap-3"><h3 className="section-label m-0">Tags</h3>{selectedTags.length > 0 && <button className="pack-tag-clear" onClick={() => setSelectedTags([])}><X size={11}/>Clear {selectedTags.length}</button>}</div><div className="preset-tags mt-2">{Object.entries(scopedTags).sort((a,b) => b[1]-a[1]).map(([tag,count]) => <button key={tag} className={`preset-tag pack-tag-button ${selectedTags.includes(tag) ? "pack-tag-active" : ""}`} onClick={() => toggleTag(tag)}>{tag} <small>{count}</small></button>)}</div>
    </div>
    <div className="flex items-center justify-between border-b border-white/[.07] px-6 py-3"><div><h3 className="text-sm font-semibold text-white">Pack presets <span className="ml-1 font-normal text-slate-600">{presets.length}</span></h3><p className="mt-0.5 text-xs text-slate-600">{selectedTags.length ? `Filtered by ${selectedTags.length} tag${selectedTags.length === 1 ? "" : "s"}` : "Used presets are highlighted"}</p></div><label className="flex items-center gap-2 text-xs text-slate-400"><input type="checkbox" checked={usedOnly} onChange={(event) => setUsedOnly(event.target.checked)}/>Used only</label></div>
    <div className="overflow-auto">
      <table className="pack-presets-table w-full table-fixed border-collapse"><thead><tr><th>Preset</th><th>Bank/Slot</th><th>Tags</th></tr></thead><tbody>{presets.map((preset, index) => <tr key={`${preset.name}-${index}`} className={preset.used ? "pack-preset-used" : ""}><td><div className="pack-preset-name"><span className="min-w-0 truncate uppercase" title={`${preset.name}.${preset.fileType.toLocaleLowerCase()}`}>{preset.name}<small className="preset-type">.{preset.fileType.toLocaleLowerCase()}</small></span></div></td><td className={`whitespace-nowrap ${preset.used ? "font-semibold text-emerald-300" : "text-slate-700"}`}>{preset.locations.join(", ") || "—"}</td><td><div className="preset-tags">{preset.tags.map((tag) => <span className="preset-tag" key={tag}>{tag}</span>)}</div></td></tr>)}</tbody></table>
    </div>
  </div>;
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
function BackgroundSyncStatus({ progress, paused, onTogglePause }: { progress: SyncProgress; paused: boolean; onTogglePause: () => void }) {
  const location = progress.bank ? progress.slot ? `${progress.bank} ${String(progress.slot).padStart(3, "0")}` : progress.bank : progress.stage;
  return <div className="grid w-[30vw] grid-cols-[minmax(0,1fr)_90px_auto_auto] items-center gap-2 px-3"><div className="flex min-w-0 items-center gap-1.5 text-[11px]"><RefreshCw size={12} className={`shrink-0 text-emerald-400 ${paused ? "" : "animate-spin"}`}/><span className="shrink-0 font-medium text-slate-300">{paused ? "Paused" : "Sync"}</span><span className="truncate text-slate-600">{location}</span></div><div className="flex items-center gap-1.5"><div className="h-1 flex-1 overflow-hidden rounded-full bg-white/[.08]"><div className={`h-full rounded-full transition-[width] duration-300 ${paused ? "bg-amber-400" : "bg-emerald-400"}`} style={{ width: `${progress.percent}%` }}/></div><span className={`w-7 text-right text-[10px] font-semibold ${paused ? "text-amber-300" : "text-emerald-300"}`}>{progress.percent}%</span></div><div className="whitespace-nowrap text-[10px] text-slate-500">{progress.total ? `${progress.completed}/${progress.total}` : "Catalog…"}</div><button className={`sync-pause-button ${paused ? "sync-resume-button" : ""}`} onClick={onTogglePause}>{paused ? <Play size={10}/> : <Pause size={10}/>} {paused ? "RESUME" : "PAUSE"}</button></div>;
}
function SyncError({ detail, onClose, onRetry }: { detail: string; onClose: () => void; onRetry: () => void }) { return <div className="modal-backdrop"><div className="modal-panel"><button className="absolute right-5 top-5 text-slate-600 hover:text-white" onClick={onClose}><X size={18}/></button><div className="modal-icon border-red-400/20 bg-red-400/10 text-red-300"><X/></div><p className="eyebrow text-red-300">Synchronization failed</p><h2 className="modal-title">The device could not be read</h2><p className="modal-copy">No local preset files were changed.</p><pre className="mt-6 max-h-40 overflow-auto whitespace-pre-wrap rounded-lg border border-white/[.08] bg-black/30 p-4 text-left text-xs text-red-200">{detail}</pre><div className="mt-6 flex gap-3"><button className="secondary-button flex-1 justify-center" onClick={onClose}>Close</button><button className="primary-button flex-1 justify-center" onClick={onRetry}><RefreshCw size={16}/>Retry</button></div></div></div>; }

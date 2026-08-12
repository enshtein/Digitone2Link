export type Preset = { bank:string; slot:number; name:string; tags:string[]; exactPacks:string[]; nameOnlyPacks:string[]; duplicateLocations:string[]; error:string|null };
export type Pack = { name:string; total:number; found:number; exact:number; nameOnly:number; tags:Record<string,number>; matches:{location:string;name:string}[] };
export type ScanResult = { banks:Record<string,Preset[]>; packs:Pack[]; errors:string[] };
export type Settings = { banksPath:string|null; packsPath:string|null };

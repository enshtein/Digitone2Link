# Digitone Presets

Desktop library for browsing and analysing Elektron Digitone II presets. This is
the Tauri rewrite of Digitone II Preset Library, with a React/TypeScript UI and a
Rust filesystem engine.

## Stack

- Tauri 2
- React 19 + TypeScript
- Tailwind CSS
- Rust

## Development

Prerequisites: Node.js, npm, Rust, and the platform dependencies required by
Tauri.

```sh
npm install
npm run tauri dev
```

Build the frontend and desktop bundle:

```sh
npm run build
npm run tauri build
```

Preset files stay on your computer. The app reads `.dn2pst` and `.dnsnd`
containers, fingerprints their payloads, and stores only the two selected folder
paths in the operating system's application settings directory.

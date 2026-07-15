# Host requirements

The minimum toolchain needed to build and run Talon from source.

## Core

- **Rust ≥ 1.78** (`rustup update stable`)
- **Node ≥ 22.13** (required by pnpm 11; LTS recommended)
- **pnpm ≥ 11** (`npm i -g pnpm` or via Corepack)
- **Tauri CLI 2.x** (`cargo install tauri-cli --version "^2.0" --locked`)

## Linux extra packages (Debian/Ubuntu)

```bash
sudo apt install -y \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  patchelf
```

Optional for headless smoke tests:

```bash
sudo apt install -y xvfb
```

## macOS

- **Xcode Command Line Tools** (`xcode-select --install`)

## Windows

- **WebView2 runtime** (preinstalled on Windows 11; on Windows 10 the Tauri build downloads it)
- **MSVC build tools** (Visual Studio Build Tools with the "Desktop development with C++" workload)

## Verifying

```bash
rustc --version   # expect 1.78 or newer
cargo --version
node --version    # expect v22.13 or newer
pnpm --version    # expect 11 or newer
cargo tauri --version   # expect 2.x after install
```

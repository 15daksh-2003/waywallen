<p align="center">
  <img src="ui/assets/waywallen-ui.svg" alt="Waywallen" width="128" />
</p>

<h1 align="center">Waywallen</h1>

<p align="center"><strong> Wallpaper Manager for Linux </strong></p>

<a href="README.CN.md">中文 README</a> · <a href="https://discord.gg/2xEdmMrhRF">Discord</a>

---

Waywallen is a dynamic wallpaper solution for Linux desktops.  
It started life as a Wallpaper Engine plugin for KDE.

---

## Screenshots

<p align="center">
  <img src="ui/assets/main_page.webp" alt="Waywallen main page" width="720" />
</p>

## Quick Start

### Install

**Prebuilt binaries** — grab the latest appimage from the [Releases page](https://github.com/waywallen/waywallen/releases).

**Flatpak**  

<a href='https://flathub.org/en/apps/org.waywallen.waywallen'>
<img width='240' alt='Get it on Flathub' src='https://flathub.org/api/badge?locale=en'/>
</a>

**From source** — see [BUILD.md](BUILD.md).

### Desktop integration

| Desktop | Integration | Mouse input | Auto pause |
|---------|-------------|:-----------:|:----------:|
| **KDE Plasma** | [waywallen-display](https://github.com/waywallen/waywallen-display/) | ✅ | ✅ |
| **GNOME** | [waywallen-display](https://github.com/waywallen/waywallen-display/) | ✅ | ✅ |
| **Hyprland** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ✅ |
| **Niri** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ❌ |
| **Sway** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ❌ |
| **COSMIC** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ❌ |

## Known issue
- Web wallpapers on nvidia gpu require to set `shared_texture_enabled` OFF in web renderer setting.

## Wallpaper plugins
- image plugin
- video plugin
  - hwdec by vulkan,vaapi
- wallhaven plugin

### Third plugins
- [open-wallpaper-engine](https://github.com/waywallen/open-wallpaper-engine)
  - scene support
  - web support

> [!NOTE]  
> For third plugins:  
> You need to mannually download plugin zip and install in the ui's plugins page.  
> After installed, this plugin's update will be notified and handled by waywallen.  

## FAQ
- How to get logs  
  You must exit the pre-launched waywallen daemon.  
  ```bash
  export RSTD_LOG=debug RUST_LOG=debug,zbus=warn
  ./waywallen
  ```
- How to debug in flatpak
  ```bash
  flatpak install org.waywallen.waywallen.Debug
  flatpak run --devel --command=bash org.waywallen.waywallen
  # 1. run directly
  [📦 org.waywallen.waywallen ~]$ gdb waywallen
  (gdb) run
  Enable debuginfod for this session? (y or [n]) n
  ...
  # get the stacktrace
  (gdb) bt
  
  # 2. or use coredump file
  coredumpctl dump <id> -o core.save
  flatpak run --devel --filesystem=host --command=bash org.waywallen.waywallen
  [📦 org.waywallen.waywallen ~]$ gdb waywallen core.save
  ...
  ```

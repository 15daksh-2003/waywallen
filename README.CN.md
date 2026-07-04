<p align="center">
  <img src="ui/assets/waywallen-ui.svg" alt="Waywallen" width="128" />
</p>

<h1 align="center">Waywallen</h1>

<p align="center"><strong> Wallpaper Manager for Linux </strong></p>

<a href="README.md">English README</a> · <a href="https://discord.gg/2xEdmMrhRF">Discord</a>

---

Waywallen 是一个为 Linux 桌面打造的动态壁纸方案  
最初是 wallpaper engine plugin for kde  

---

## 界面

<p align="center">
  <img src="ui/assets/main_page.webp" alt="Waywallen 主界面" width="720" />
</p>

## 快速开始

### 安装

**预编译包** —— 到 [Releases 页面](https://github.com/waywallen/waywallen/releases) 下载最新版本。

**Flatpak**  

<a href='https://flathub.org/en/apps/org.waywallen.waywallen'>
  <img width='240' alt='Get it on Flathub' src='https://flathub.org/api/badge?locale=zh-Hans'/>
</a>

**从源码构建** —— 见 [BUILD.md](BUILD.md)。

### 桌面集成

| 桌面 | 集成 | 鼠标输入 | 自动暂停 |
|---------|-------------|:-----------:|:----------:|
| **KDE Plasma** | [waywallen-display](https://github.com/waywallen/waywallen-display/) | ✅ | ✅ |
| **GNOME** | [waywallen-display](https://github.com/waywallen/waywallen-display/) | ✅ | ✅ |
| **Hyprland** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ✅ |
| **Niri** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ❌ |
| **Sway** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ❌ |
| **COSMIC** | [waywallen-display/layer_shell](https://github.com/waywallen/waywallen-display/tree/main/src/bin/layer_shell) | ✅ | ❌ |

## 已知问题
- Nvidia gpu 运行网页壁纸需要在 web renderer 的设置中关闭 `shared_texture_enabled`.

## 壁纸插件
- 图片插件
- 视频插件
  - 硬解：vulkan、vaapi
- wallhaven 插件

### 第三方插件
- [open-wallpaper-engine](https://github.com/waywallen/open-wallpaper-engine)
  - 场景壁纸支持
  - 网页壁纸支持

> [!NOTE]  
> 对于第三方插件：  
> 需要手动下载插件 zip，并在 UI 的插件页面中安装。  
> 安装完成后，后续该插件的更新 waywallen 会自己提示和处理。

## FAQ
- 如何在 flatpak 中调试
  ```bash
  flatpak install org.waywallen.waywallen.Debug
  flatpak run --devel --command=bash org.waywallen.waywallen
  # 1. 直接运行
  [📦 org.waywallen.waywallen ~]$ gdb Qcm
  (gdb) run
  Enable debuginfod for this session? (y or [n]) n
  ...
  # 获取堆栈
  (gdb) bt
  
  # 2. 或使用 coredump 文件
  coredumpctl dump <id> -o core.save
  flatpak run --devel --filesystem=host --command=bash org.waywallen.waywallen
  [📦 org.waywallen.waywallen ~]$ gdb Qcm core.save
  ...
  ```

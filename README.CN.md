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

## Known issue
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

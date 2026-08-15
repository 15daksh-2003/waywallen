use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

const PCI_IDS_PATHS: [&str; 3] = [
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/usr/share/pci.ids",
];

/// Plugin-settings key that persists "preferred GPU" as `"<major>:<minor>"`.
/// The daemon translates it to a `/dev/dri/renderD*` path at spawn time.
pub const GPU_DRM_DEV_KEY: &str = "gpu_drm_dev";

/// Settings key that flows to the renderer subprocess's Init.settings.
pub const RENDER_NODE_KEY: &str = "render_node";

pub fn parse_drm_dev(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

pub fn format_drm_dev(major: u32, minor: u32) -> String {
    format!("{major}:{minor}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuInfo {
    pub render_node: Option<PathBuf>,
    pub primary_node: Option<PathBuf>,
    pub render_major: u32,
    pub render_minor: u32,
    pub primary_major: u32,
    pub primary_minor: u32,
    pub pci_bdf: Option<String>,
    pub vendor_id: u16,
    pub device_id: u16,
    pub subsystem_vendor_id: u16,
    pub subsystem_device_id: u16,
    pub driver: String,
    pub name: String,
    pub description: String,
}

impl GpuInfo {
    pub fn matches_render(&self, major: u32, minor: u32) -> bool {
        self.render_node.is_some() && self.render_major == major && self.render_minor == minor
    }
}

pub fn enumerate() -> Vec<GpuInfo> {
    let mut gpus = enumerate_with_roots(Path::new("/dev/dri"), Path::new("/sys/dev/char"));
    if let Some(path) = resolve_pci_names_from_paths(&mut gpus, &PCI_IDS_PATHS) {
        log::debug!("system::gpu: resolved PCI names from {}", path.display());
    }
    finish_descriptions(&mut gpus);
    gpus
}

pub(crate) fn enumerate_with_roots(dev_dri: &Path, sysfs_char: &Path) -> Vec<GpuInfo> {
    let entries = match fs::read_dir(dev_dri) {
        Ok(it) => it,
        Err(e) => {
            log::warn!(
                "system::gpu::enumerate: read_dir({}) failed: {e}",
                dev_dri.display()
            );
            return Vec::new();
        }
    };

    // Group by PCI device directory so a single GPU's cardN and
    // renderD1xx nodes collapse to one GpuInfo.
    let mut groups: BTreeMap<String, GpuInfo> = BTreeMap::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let kind = if name.starts_with("renderD") {
            NodeKind::Render
        } else if name.starts_with("card") {
            NodeKind::Primary
        } else {
            continue;
        };

        let (major, minor) = match stat_rdev(&path) {
            Some(t) => t,
            None => continue,
        };

        let pci = read_pci_for_node(sysfs_char, major, minor);
        let group_key = pci
            .as_ref()
            .and_then(|p| p.dir.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| path.display().to_string());

        let g = groups.entry(group_key).or_default();
        match kind {
            NodeKind::Render => {
                g.render_node = Some(path.clone());
                g.render_major = major;
                g.render_minor = minor;
            }
            NodeKind::Primary => {
                g.primary_node = Some(path.clone());
                g.primary_major = major;
                g.primary_minor = minor;
            }
        }
        if let Some(p) = pci {
            // Card + render in the same group resolve to the same PCI dir,
            // so overwriting is a no-op the second time around.
            g.pci_bdf = Some(p.bdf);
            g.vendor_id = p.vendor;
            g.device_id = p.device;
            g.subsystem_vendor_id = p.subsystem_vendor;
            g.subsystem_device_id = p.subsystem_device;
            g.driver = p.driver;
            if g.name.is_empty() {
                g.name = p.product_name.unwrap_or_default();
            }
        }
    }

    let mut out: Vec<GpuInfo> = groups.into_values().collect();
    // Stable order for UI: entries with a render node first, then by
    // render minor / primary minor.
    out.sort_by_key(|g| (g.render_node.is_none(), g.render_minor, g.primary_minor));
    out
}

enum NodeKind {
    Render,
    Primary,
}

struct Pci {
    dir: PathBuf,
    bdf: String,
    vendor: u16,
    device: u16,
    subsystem_vendor: u16,
    subsystem_device: u16,
    driver: String,
    product_name: Option<String>,
}

fn stat_rdev(p: &Path) -> Option<(u32, u32)> {
    let st = nix::sys::stat::stat(p).ok()?;
    let rdev = st.st_rdev as u64;
    Some((dev_major(rdev), dev_minor(rdev)))
}

// Linux glibc dev_t encoding (extended)
fn dev_major(rdev: u64) -> u32 {
    (((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfffu64)) as u32
}
fn dev_minor(rdev: u64) -> u32 {
    ((rdev & 0xff) | ((rdev >> 12) & !0xffu64)) as u32
}

fn read_pci_for_node(sysfs_char: &Path, major: u32, minor: u32) -> Option<Pci> {
    let link = sysfs_char.join(format!("{major}:{minor}")).join("device");
    parse_pci_dir(&link)
}

fn parse_pci_dir(device_link: &Path) -> Option<Pci> {
    let dir = fs::canonicalize(device_link).ok()?;
    let bdf = dir.file_name()?.to_str()?.to_string();

    let vendor = read_hex_u16(&dir.join("vendor"))?;
    let device = read_hex_u16(&dir.join("device"))?;
    let subsystem_vendor = read_hex_u16(&dir.join("subsystem_vendor")).unwrap_or_default();
    let subsystem_device = read_hex_u16(&dir.join("subsystem_device")).unwrap_or_default();
    let driver = read_driver(&dir).unwrap_or_default();
    let product_name = read_trimmed(&dir.join("product_name"));

    Some(Pci {
        dir,
        bdf,
        vendor,
        device,
        subsystem_vendor,
        subsystem_device,
        driver,
        product_name,
    })
}

fn read_trimmed(path: &Path) -> Option<String> {
    let value = fs::read_to_string(path).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn read_hex_u16(p: &Path) -> Option<u16> {
    let s = fs::read_to_string(p).ok()?;
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    u16::from_str_radix(s, 16).ok()
}

fn read_driver(pci_dir: &Path) -> Option<String> {
    // /sys/.../device/driver -> ../../bus/pci/drivers/<name>
    let target = fs::read_link(pci_dir.join("driver")).ok()?;
    Some(target.file_name()?.to_str()?.to_string())
}

#[derive(Debug)]
struct PciNameTarget {
    gpu_index: usize,
    vendor: u16,
    device: u16,
    subsystem_vendor: u16,
    subsystem_device: u16,
    device_name: Option<String>,
    subsystem_name: Option<String>,
}

fn resolve_pci_names_from_paths<P: AsRef<Path>>(
    gpus: &mut [GpuInfo],
    paths: &[P],
) -> Option<PathBuf> {
    if !gpus
        .iter()
        .any(|gpu| gpu.name.is_empty() && gpu.vendor_id != 0 && gpu.device_id != 0)
    {
        return None;
    }

    for candidate in paths {
        let path = candidate.as_ref();
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) => {
                log::debug!("system::gpu: open {}: {error}", path.display());
                continue;
            }
        };
        if let Err(error) = resolve_pci_names(gpus, BufReader::new(file)) {
            log::warn!("system::gpu: read {}: {error}", path.display());
        }
        return Some(path.to_path_buf());
    }
    None
}

fn resolve_pci_names<R: BufRead>(gpus: &mut [GpuInfo], mut reader: R) -> io::Result<()> {
    let mut targets: Vec<PciNameTarget> = gpus
        .iter()
        .enumerate()
        .filter(|(_, gpu)| gpu.name.is_empty() && gpu.vendor_id != 0 && gpu.device_id != 0)
        .map(|(gpu_index, gpu)| PciNameTarget {
            gpu_index,
            vendor: gpu.vendor_id,
            device: gpu.device_id,
            subsystem_vendor: gpu.subsystem_vendor_id,
            subsystem_device: gpu.subsystem_device_id,
            device_name: None,
            subsystem_name: None,
        })
        .collect();
    if targets.is_empty() {
        return Ok(());
    }

    let mut current_vendor = None;
    let mut current_device = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("C ") {
            current_vendor = None;
            current_device = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("\t\t") {
            let Some(vendor) = take_hex_field(rest) else {
                continue;
            };
            let Some(device) = take_hex_field(vendor.1) else {
                continue;
            };
            let name = device.1.trim();
            if name.is_empty() {
                continue;
            }
            let (Some(parent_vendor), Some(parent_device)) = (current_vendor, current_device)
            else {
                continue;
            };
            for target in &mut targets {
                if target.vendor == parent_vendor
                    && target.device == parent_device
                    && target.subsystem_vendor != 0
                    && target.subsystem_device != 0
                    && target.subsystem_vendor == vendor.0
                    && target.subsystem_device == device.0
                {
                    target.subsystem_name = Some(name.to_owned());
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix('\t') {
            let Some((device, name)) = take_hex_field(rest) else {
                continue;
            };
            current_device = Some(device);
            let Some(vendor) = current_vendor else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            for target in &mut targets {
                if target.vendor == vendor && target.device == device {
                    target.device_name = Some(name.to_owned());
                }
            }
            continue;
        }

        current_device = None;
        current_vendor = take_hex_field(line).map(|(vendor, _)| vendor);
    }

    for target in targets {
        if let Some(name) = target.subsystem_name.or(target.device_name) {
            gpus[target.gpu_index].name = name;
        }
    }
    Ok(())
}

fn take_hex_field(value: &str) -> Option<(u16, &str)> {
    let value = value.trim_start();
    let end = value.find(char::is_whitespace)?;
    if end != 4 {
        return None;
    }
    let id = u16::from_str_radix(&value[..end], 16).ok()?;
    Some((id, value[end..].trim_start()))
}

fn finish_descriptions(gpus: &mut [GpuInfo]) {
    for gpu in gpus {
        gpu.description = format_description(gpu);
    }
}

fn format_description(g: &GpuInfo) -> String {
    let driver = if g.driver.is_empty() {
        "unknown".to_string()
    } else {
        g.driver.clone()
    };
    let identity = if g.vendor_id == 0 && g.device_id == 0 {
        driver
    } else {
        format!("{driver} {:#06x}:{:#06x}", g.vendor_id, g.device_id)
    };
    if g.name.is_empty() {
        identity
    } else {
        format!("{} — {identity}", g.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::fs::symlink;

    /// Fake sysfs: an empty renderD128 in dev_dri, sysfs_char/<m>:<n>/device
    /// symlinked to a PCI dir with vendor/device/driver populated. mknod
    #[test]
    fn parse_pci_dir_reads_vendor_device_driver() {
        let tmp = tempfile::tempdir().unwrap();
        let pci = tmp.path().join("0000:03:00.0");
        fs::create_dir_all(&pci).unwrap();
        fs::write(pci.join("vendor"), "0x1002\n").unwrap();
        fs::write(pci.join("device"), "0x73bf\n").unwrap();
        fs::write(pci.join("subsystem_vendor"), "0x1da2\n").unwrap();
        fs::write(pci.join("subsystem_device"), "0xe471\n").unwrap();
        fs::write(pci.join("product_name"), "Radeon Test GPU\n").unwrap();
        let drivers = tmp.path().join("drivers/amdgpu");
        fs::create_dir_all(&drivers).unwrap();
        symlink(&drivers, pci.join("driver")).unwrap();

        let chardir = tmp.path().join("226-128");
        fs::create_dir_all(&chardir).unwrap();
        let device_link = chardir.join("device");
        symlink(&pci, &device_link).unwrap();

        let p = parse_pci_dir(&device_link).expect("parse");
        assert_eq!(p.bdf, "0000:03:00.0");
        assert_eq!(p.vendor, 0x1002);
        assert_eq!(p.device, 0x73bf);
        assert_eq!(p.subsystem_vendor, 0x1da2);
        assert_eq!(p.subsystem_device, 0xe471);
        assert_eq!(p.driver, "amdgpu");
        assert_eq!(p.product_name.as_deref(), Some("Radeon Test GPU"));
    }

    #[test]
    fn resolve_pci_names_matches_multiple_devices_in_one_scan() {
        let database = "\
# comment
1002  Advanced Micro Devices, Inc. [AMD/ATI]
\t13c0  Granite Ridge [Radeon Graphics]
\t7550  Navi 48 [Radeon RX 9070/9070 XT/9070 GRE]
\t\t1da2 e471  Radeon RX 9070 XT Board
C 03  Display controller
\t00  VGA compatible controller
10DE  NVIDIA Corporation
\t2188  TU116 [GeForce GTX 1650]
";
        let mut gpus = vec![
            GpuInfo {
                vendor_id: 0x1002,
                device_id: 0x7550,
                subsystem_vendor_id: 0x1da2,
                subsystem_device_id: 0xe471,
                ..Default::default()
            },
            GpuInfo {
                vendor_id: 0x10de,
                device_id: 0x2188,
                ..Default::default()
            },
            GpuInfo {
                vendor_id: 0x1002,
                device_id: 0xffff,
                ..Default::default()
            },
        ];

        resolve_pci_names(&mut gpus, Cursor::new(database)).unwrap();

        assert_eq!(gpus[0].name, "Radeon RX 9070 XT Board");
        assert_eq!(gpus[1].name, "TU116 [GeForce GTX 1650]");
        assert!(gpus[2].name.is_empty());
    }

    #[test]
    fn resolve_pci_names_uses_only_first_available_database() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing.ids");
        let selected = tmp.path().join("selected.ids");
        let ignored = tmp.path().join("ignored.ids");
        fs::write(&selected, "1002  AMD\n\t7550  Selected name\n").unwrap();
        fs::write(&ignored, "1002  AMD\n\t7550  Ignored name\n").unwrap();
        let mut gpus = vec![GpuInfo {
            vendor_id: 0x1002,
            device_id: 0x7550,
            ..Default::default()
        }];

        let path = resolve_pci_names_from_paths(&mut gpus, &[missing, selected.clone(), ignored]);

        assert_eq!(path.as_deref(), Some(selected.as_path()));
        assert_eq!(gpus[0].name, "Selected name");
    }

    #[test]
    fn resolve_pci_names_preserves_existing_name_without_database() {
        let tmp = tempfile::tempdir().unwrap();
        let mut gpus = vec![
            GpuInfo {
                vendor_id: 0x1002,
                device_id: 0x7550,
                name: "Hardware product name".to_string(),
                ..Default::default()
            },
            GpuInfo {
                vendor_id: 0x10de,
                device_id: 0x2188,
                ..Default::default()
            },
        ];

        let path = resolve_pci_names_from_paths(&mut gpus, &[tmp.path().join("missing.ids")]);

        assert!(path.is_none());
        assert_eq!(gpus[0].name, "Hardware product name");
        assert!(gpus[1].name.is_empty());
    }

    #[test]
    fn dev_major_minor_round_trip() {
        // makedev(226, 128) on Linux extended encoding = (226 << 8) | 128
        let rdev: u64 = (226u64 << 8) | 128u64;
        assert_eq!(dev_major(rdev), 226);
        assert_eq!(dev_minor(rdev), 128);
    }

    #[test]
    fn format_description_handles_unknown_pci() {
        let g = GpuInfo {
            driver: "vgem".to_string(),
            ..Default::default()
        };
        assert_eq!(format_description(&g), "vgem");
    }

    #[test]
    fn format_description_includes_resolved_name() {
        let g = GpuInfo {
            vendor_id: 0x1002,
            device_id: 0x7550,
            driver: "amdgpu".to_string(),
            name: "Navi 48".to_string(),
            ..Default::default()
        };
        assert_eq!(format_description(&g), "Navi 48 — amdgpu 0x1002:0x7550");
    }

    #[test]
    fn matches_render_requires_render_node() {
        let mut g = GpuInfo {
            render_major: 226,
            render_minor: 128,
            ..Default::default()
        };
        assert!(!g.matches_render(226, 128));
        g.render_node = Some(PathBuf::from("/dev/dri/renderD128"));
        assert!(g.matches_render(226, 128));
        assert!(!g.matches_render(226, 129));
    }

    #[test]
    #[ignore = "live: requires /dev/dri/renderD128"]
    fn live_enumerate_finds_a_gpu() {
        let v = enumerate();
        assert!(!v.is_empty(), "expected at least one GPU on this host");
        let any_render = v.iter().any(|g| g.render_node.is_some());
        assert!(any_render, "expected at least one render node");
    }
}

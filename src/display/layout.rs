use serde::{Deserialize, Serialize};

use crate::scheduler::ProjectedConfig;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FillMode {
    Stretched,
    PreserveAspectFit,
    #[default]
    PreserveAspectCrop,
    Centered,
}

/// Buffer-side rotation, expressed as a clockwise turn of the displayed
/// image. Wire-mapped onto `wl_output.transform` 0..3.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    #[default]
    Normal,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    /// `wl_output.transform` value matching this rotation. The
    /// compositor reads this from `set_buffer_transform`.
    pub fn to_wl_transform(self) -> u32 {
        match self {
            Rotation::Normal => 0,
            Rotation::Cw90 => 1,
            Rotation::Cw180 => 2,
            Rotation::Cw270 => 3,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Align {
    fn h_factor(self) -> f32 {
        match self {
            Align::TopLeft | Align::Left | Align::BottomLeft => 0.0,
            Align::Top | Align::Center | Align::Bottom => 0.5,
            Align::TopRight | Align::Right | Align::BottomRight => 1.0,
        }
    }
    fn v_factor(self) -> f32 {
        match self {
            Align::TopLeft | Align::Top | Align::TopRight => 0.0,
            Align::Left | Align::Center | Align::Right => 0.5,
            Align::BottomLeft | Align::Bottom | Align::BottomRight => 1.0,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Location {
    pub x: u8,
    pub y: u8,
}

impl Default for Location {
    fn default() -> Self {
        Self { x: 50, y: 50 }
    }
}

impl Location {
    pub fn new(x: u8, y: u8) -> Self {
        Self {
            x: x.min(100),
            y: y.min(100),
        }
    }

    pub fn from_align(align: Align) -> Self {
        Self::new(
            (align.h_factor() * 100.0).round() as u8,
            (align.v_factor() * 100.0).round() as u8,
        )
    }

    pub fn to_align(self) -> Align {
        fn bucket(v: u8) -> u8 {
            if v <= 25 {
                0
            } else if v >= 75 {
                2
            } else {
                1
            }
        }
        match (bucket(self.x), bucket(self.y)) {
            (0, 0) => Align::TopLeft,
            (1, 0) => Align::Top,
            (2, 0) => Align::TopRight,
            (0, 1) => Align::Left,
            (1, 1) => Align::Center,
            (2, 1) => Align::Right,
            (0, 2) => Align::BottomLeft,
            (1, 2) => Align::Bottom,
            (2, 2) => Align::BottomRight,
            _ => Align::Center,
        }
    }

    fn h_factor(self) -> f32 {
        f32::from(self.x.min(100)) / 100.0
    }

    fn v_factor(self) -> f32 {
        f32::from(self.y.min(100)) / 100.0
    }
}

#[derive(Copy, Clone, Debug)]
pub struct LayoutInput {
    pub tex_w: f32,
    pub tex_h: f32,
    pub disp_w: f32,
    pub disp_h: f32,
    pub fillmode: FillMode,
    pub location: Location,
    pub clear_rgba: [f32; 4],
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LayoutOutput {
    /// Source rect in texture pixels: (x, y, w, h).
    pub source: (f32, f32, f32, f32),
    /// Destination rect in display pixels: (x, y, w, h).
    pub dest: (f32, f32, f32, f32),
    /// Background fill color (RGBA, sRGB straight alpha).
    pub clear_rgba: [f32; 4],
}

/// Resolve one layout. Pure; never panics. Degenerate inputs
/// collapse to a Stretched output with clamped non-negative dimensions.
pub fn compute(i: LayoutInput) -> LayoutOutput {
    if i.tex_w <= 0.0 || i.tex_h <= 0.0 || i.disp_w <= 0.0 || i.disp_h <= 0.0 {
        return LayoutOutput {
            source: (0.0, 0.0, i.tex_w.max(0.0), i.tex_h.max(0.0)),
            dest: (0.0, 0.0, i.disp_w.max(0.0), i.disp_h.max(0.0)),
            clear_rgba: i.clear_rgba,
        };
    }

    match i.fillmode {
        FillMode::Stretched => LayoutOutput {
            source: (0.0, 0.0, i.tex_w, i.tex_h),
            dest: (0.0, 0.0, i.disp_w, i.disp_h),
            clear_rgba: i.clear_rgba,
        },

        FillMode::PreserveAspectFit => {
            let scale = (i.disp_w / i.tex_w).min(i.disp_h / i.tex_h);
            let dw = i.tex_w * scale;
            let dh = i.tex_h * scale;
            let dx = (i.disp_w - dw) * i.location.h_factor();
            let dy = (i.disp_h - dh) * i.location.v_factor();
            LayoutOutput {
                source: (0.0, 0.0, i.tex_w, i.tex_h),
                dest: (dx, dy, dw, dh),
                clear_rgba: i.clear_rgba,
            }
        }

        FillMode::PreserveAspectCrop => {
            // Pick a source rect that preserves aspect when stretched
            // to the full display.
            let scale = (i.disp_w / i.tex_w).max(i.disp_h / i.tex_h);
            let sw = i.disp_w / scale;
            let sh = i.disp_h / scale;
            let sx = (i.tex_w - sw) * i.location.h_factor();
            let sy = (i.tex_h - sh) * i.location.v_factor();
            LayoutOutput {
                source: (sx, sy, sw, sh),
                dest: (0.0, 0.0, i.disp_w, i.disp_h),
                clear_rgba: i.clear_rgba,
            }
        }

        FillMode::Centered => {
            // Display texture pixels 1:1, centering or cropping per axis
            // according to the requested location.
            let (sx, sw, dx, dw) = axis_centered(i.tex_w, i.disp_w, i.location.h_factor());
            let (sy, sh, dy, dh) = axis_centered(i.tex_h, i.disp_h, i.location.v_factor());
            LayoutOutput {
                source: (sx, sy, sw, sh),
                dest: (dx, dy, dw, dh),
                clear_rgba: i.clear_rgba,
            }
        }
    }
}

/// Map an actual display-surface point to renderer-texture-local pixels.
/// The destination rectangle is in pre-transform display space, so the
/// surface point must be transformed back before applying that rectangle.
pub fn display_point_to_texture(
    disp_x: f32,
    disp_y: f32,
    cfg: &ProjectedConfig,
) -> Option<(f32, f32)> {
    if cfg.display_w <= 0.0 || cfg.display_h <= 0.0 || cfg.dest_w <= 0.0 || cfg.dest_h <= 0.0 {
        return None;
    }

    let display_u = disp_x / cfg.display_w;
    let display_v = disp_y / cfg.display_h;
    if !(0.0..=1.0).contains(&display_u) || !(0.0..=1.0).contains(&display_v) {
        return None;
    }

    let (pre_u, pre_v) = match cfg.transform {
        0 => (display_u, display_v),
        1 => (display_v, 1.0 - display_u),
        2 => (1.0 - display_u, 1.0 - display_v),
        3 => (1.0 - display_v, display_u),
        4 => (1.0 - display_u, display_v),
        5 => (display_v, display_u),
        6 => (display_u, 1.0 - display_v),
        7 => (1.0 - display_v, 1.0 - display_u),
        _ => (display_u, display_v),
    };

    let swaps_dimensions = matches!(cfg.transform, 1 | 3 | 5 | 7);
    let (pre_w, pre_h) = if swaps_dimensions {
        (cfg.display_h, cfg.display_w)
    } else {
        (cfg.display_w, cfg.display_h)
    };
    let pre_x = pre_u * pre_w;
    let pre_y = pre_v * pre_h;
    let source_u = (pre_x - cfg.dest_x) / cfg.dest_w;
    let source_v = (pre_y - cfg.dest_y) / cfg.dest_h;
    if !(0.0..=1.0).contains(&source_u) || !(0.0..=1.0).contains(&source_v) {
        return None;
    }

    Some((
        cfg.source_x + source_u * cfg.source_w,
        cfg.source_y + source_v * cfg.source_h,
    ))
}

/// One axis of `Centered`. Returns `(source_off, source_len, dest_off, dest_len)`.
fn axis_centered(tex: f32, disp: f32, factor: f32) -> (f32, f32, f32, f32) {
    if tex <= disp {
        // Texture fits — place fully inside the display.
        let dest_len = tex;
        let dest_off = (disp - tex) * factor;
        (0.0, tex, dest_off, dest_len)
    } else {
        // Texture is larger than display — crop a viewport of `disp`
        // pixels out of the texture, positioned by `factor`.
        let src_off = (tex - disp) * factor;
        (src_off, disp, 0.0, disp)
    }
}

// ---------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn input(tex: (f32, f32), disp: (f32, f32), fillmode: FillMode, align: Align) -> LayoutInput {
        LayoutInput {
            tex_w: tex.0,
            tex_h: tex.1,
            disp_w: disp.0,
            disp_h: disp.1,
            fillmode,
            location: Location::from_align(align),
            clear_rgba: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn input_at(
        tex: (f32, f32),
        disp: (f32, f32),
        fillmode: FillMode,
        location: Location,
    ) -> LayoutInput {
        LayoutInput {
            tex_w: tex.0,
            tex_h: tex.1,
            disp_w: disp.0,
            disp_h: disp.1,
            fillmode,
            location,
            clear_rgba: [0.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn stretched_is_identity_regardless_of_align() {
        let out = compute(input(
            (1920.0, 1080.0),
            (1280.0, 720.0),
            FillMode::Stretched,
            Align::TopLeft,
        ));
        assert_eq!(out.source, (0.0, 0.0, 1920.0, 1080.0));
        assert_eq!(out.dest, (0.0, 0.0, 1280.0, 720.0));
        let out2 = compute(input(
            (1920.0, 1080.0),
            (1280.0, 720.0),
            FillMode::Stretched,
            Align::BottomRight,
        ));
        assert_eq!(out, out2);
    }

    #[test]
    fn fit_wider_texture_letterboxes_top_bottom() {
        // 16:9 texture into 4:3 display => bars top/bottom, dest_w == disp_w
        let out = compute(input(
            (1920.0, 1080.0),
            (800.0, 600.0),
            FillMode::PreserveAspectFit,
            Align::Center,
        ));
        assert_eq!(out.source, (0.0, 0.0, 1920.0, 1080.0));
        // scale = min(800/1920, 600/1080) = min(0.4167, 0.5556) = 0.4167
        // dest_w = 1920 * 0.4167 = 800; dest_h = 1080 * 0.4167 = 450
        assert!((out.dest.0 - 0.0).abs() < 1e-3);
        assert!((out.dest.1 - 75.0).abs() < 1e-3);
        assert!((out.dest.2 - 800.0).abs() < 1e-3);
        assert!((out.dest.3 - 450.0).abs() < 1e-3);
    }

    #[test]
    fn fit_top_left_align_pins_to_corner() {
        let out = compute(input(
            (1920.0, 1080.0),
            (800.0, 600.0),
            FillMode::PreserveAspectFit,
            Align::TopLeft,
        ));
        assert!((out.dest.0 - 0.0).abs() < 1e-3);
        assert!((out.dest.1 - 0.0).abs() < 1e-3);
    }

    #[test]
    fn crop_wider_texture_crops_horizontally() {
        // 16:9 tex into 4:3 disp: scale = max(800/1920, 600/1080) = max(0.417, 0.556) = 0.556
        // sw = 800/0.556 = 1440, sh = 600/0.556 = 1080
        let out = compute(input(
            (1920.0, 1080.0),
            (800.0, 600.0),
            FillMode::PreserveAspectCrop,
            Align::Center,
        ));
        assert!((out.source.0 - 240.0).abs() < 1e-3);
        assert!((out.source.1 - 0.0).abs() < 1e-3);
        assert!((out.source.2 - 1440.0).abs() < 1e-3);
        assert!((out.source.3 - 1080.0).abs() < 1e-3);
        assert_eq!(out.dest, (0.0, 0.0, 800.0, 600.0));
    }

    #[test]
    fn crop_top_left_align_keeps_top_left_of_texture() {
        let out = compute(input(
            (1920.0, 1080.0),
            (800.0, 600.0),
            FillMode::PreserveAspectCrop,
            Align::TopLeft,
        ));
        assert!((out.source.0 - 0.0).abs() < 1e-3);
        assert!((out.source.1 - 0.0).abs() < 1e-3);
    }

    #[test]
    fn crop_fine_location_positions_visible_window() {
        let out = compute(input_at(
            (1920.0, 1080.0),
            (800.0, 600.0),
            FillMode::PreserveAspectCrop,
            Location::new(25, 50),
        ));
        assert!((out.source.0 - 120.0).abs() < 1e-3);
        assert!((out.source.1 - 0.0).abs() < 1e-3);
    }

    #[test]
    fn centered_smaller_texture_letterboxes_around_native_size() {
        // 800x600 tex into 1920x1080 disp, Center align.
        // dest_x = (1920-800)*0.5 = 560, dest_y = (1080-600)*0.5 = 240
        let out = compute(input(
            (800.0, 600.0),
            (1920.0, 1080.0),
            FillMode::Centered,
            Align::Center,
        ));
        assert_eq!(out.source, (0.0, 0.0, 800.0, 600.0));
        assert!((out.dest.0 - 560.0).abs() < 1e-3);
        assert!((out.dest.1 - 240.0).abs() < 1e-3);
        assert!((out.dest.2 - 800.0).abs() < 1e-3);
        assert!((out.dest.3 - 600.0).abs() < 1e-3);
    }

    #[test]
    fn centered_larger_texture_crops_to_display_pixel_for_pixel() {
        // 4000x3000 tex into 1920x1080 disp, Center align.
        // sx = (4000-1920)*0.5 = 1040, sy = (3000-1080)*0.5 = 960, sw=1920, sh=1080
        let out = compute(input(
            (4000.0, 3000.0),
            (1920.0, 1080.0),
            FillMode::Centered,
            Align::Center,
        ));
        assert!((out.source.0 - 1040.0).abs() < 1e-3);
        assert!((out.source.1 - 960.0).abs() < 1e-3);
        assert!((out.source.2 - 1920.0).abs() < 1e-3);
        assert!((out.source.3 - 1080.0).abs() < 1e-3);
        assert_eq!(out.dest, (0.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn centered_top_left_pins_smaller_texture_to_corner() {
        let out = compute(input(
            (800.0, 600.0),
            (1920.0, 1080.0),
            FillMode::Centered,
            Align::TopLeft,
        ));
        assert_eq!(out.dest, (0.0, 0.0, 800.0, 600.0));
    }

    #[test]
    fn degenerate_zero_input_does_not_panic() {
        let out = compute(input(
            (0.0, 0.0),
            (1920.0, 1080.0),
            FillMode::PreserveAspectFit,
            Align::Center,
        ));
        assert_eq!(out.dest, (0.0, 0.0, 1920.0, 1080.0));
        let out = compute(input(
            (1920.0, 1080.0),
            (0.0, 0.0),
            FillMode::PreserveAspectFit,
            Align::Center,
        ));
        assert_eq!(out.source, (0.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn equal_aspect_fit_and_crop_match_stretched() {
        // 16:9 into 16:9: identity for all three modes
        let s = compute(input(
            (1920.0, 1080.0),
            (3840.0, 2160.0),
            FillMode::Stretched,
            Align::Center,
        ));
        let f = compute(input(
            (1920.0, 1080.0),
            (3840.0, 2160.0),
            FillMode::PreserveAspectFit,
            Align::Center,
        ));
        let c = compute(input(
            (1920.0, 1080.0),
            (3840.0, 2160.0),
            FillMode::PreserveAspectCrop,
            Align::Center,
        ));
        assert_eq!(s.dest, f.dest);
        assert_eq!(s.dest, c.dest);
        assert_eq!(s.source, f.source);
        assert_eq!(s.source, c.source);
    }

    // -----------------------------------------------------------------
    // display_point_to_texture

    fn cfg(
        source: (f32, f32, f32, f32),
        dest: (f32, f32, f32, f32),
        transform: u32,
    ) -> ProjectedConfig {
        cfg_with_display(source, dest, (dest.0 + dest.2, dest.1 + dest.3), transform)
    }

    fn cfg_with_display(
        source: (f32, f32, f32, f32),
        dest: (f32, f32, f32, f32),
        display: (f32, f32),
        transform: u32,
    ) -> ProjectedConfig {
        ProjectedConfig {
            config_generation: 1,
            display_w: display.0,
            display_h: display.1,
            source_x: source.0,
            source_y: source.1,
            source_w: source.2,
            source_h: source.3,
            dest_x: dest.0,
            dest_y: dest.1,
            dest_w: dest.2,
            dest_h: dest.3,
            transform,
            clear_rgba: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn approx(a: (f32, f32), b: (f32, f32)) {
        let eps = 1e-3;
        assert!(
            (a.0 - b.0).abs() < eps && (a.1 - b.1).abs() < eps,
            "expected {b:?}, got {a:?}",
        );
    }

    #[test]
    fn point_identity_stretched_same_size() {
        let c = cfg((0.0, 0.0, 1920.0, 1080.0), (0.0, 0.0, 1920.0, 1080.0), 0);
        approx(
            display_point_to_texture(100.0, 50.0, &c).unwrap(),
            (100.0, 50.0),
        );
        approx(display_point_to_texture(0.0, 0.0, &c).unwrap(), (0.0, 0.0));
        approx(
            display_point_to_texture(1920.0, 1080.0, &c).unwrap(),
            (1920.0, 1080.0),
        );
    }

    #[test]
    fn point_stretched_4k_to_1080p() {
        // 4K texture stretched onto a 1080p display.
        let c = cfg((0.0, 0.0, 3840.0, 2160.0), (0.0, 0.0, 1920.0, 1080.0), 0);
        approx(
            display_point_to_texture(960.0, 540.0, &c).unwrap(),
            (1920.0, 1080.0),
        );
        approx(display_point_to_texture(0.0, 0.0, &c).unwrap(), (0.0, 0.0));
    }

    #[test]
    fn point_aspect_fit_letterbox_drops_in_bar() {
        // 1920x1080 texture into 800x600 display, fit -> dest (0, 75, 800, 450).
        let layout = compute(input(
            (1920.0, 1080.0),
            (800.0, 600.0),
            FillMode::PreserveAspectFit,
            Align::Center,
        ));
        let c = cfg(layout.source, layout.dest, 0);
        // Inside picture: center maps to texture center.
        approx(
            display_point_to_texture(400.0, 300.0, &c).unwrap(),
            (960.0, 540.0),
        );
        // Top-left of the visible picture (0, 75) maps to texture (0, 0).
        approx(display_point_to_texture(0.0, 75.0, &c).unwrap(), (0.0, 0.0));
        // In the top letterbox bar -> dropped.
        assert!(display_point_to_texture(400.0, 10.0, &c).is_none());
        // In the bottom letterbox bar -> dropped.
        assert!(display_point_to_texture(400.0, 590.0, &c).is_none());
    }

    #[test]
    fn point_aspect_crop_maps_into_visible_window() {
        // 1920x1080 tex into 800x600 disp, crop center -> source (240, 0, 1440, 1080),
        // dest (0, 0, 800, 600).
        let layout = compute(input(
            (1920.0, 1080.0),
            (800.0, 600.0),
            FillMode::PreserveAspectCrop,
            Align::Center,
        ));
        let c = cfg(layout.source, layout.dest, 0);
        // Display center maps to texture center.
        approx(
            display_point_to_texture(400.0, 300.0, &c).unwrap(),
            (960.0, 540.0),
        );
        // Top-left of display maps to top-left of the cropped source rect.
        approx(
            display_point_to_texture(0.0, 0.0, &c).unwrap(),
            (240.0, 0.0),
        );
    }

    #[test]
    fn point_outside_dest_rect_returns_none() {
        // Dest offset by (100, 50), 200x100 wide.
        let c = cfg((0.0, 0.0, 100.0, 100.0), (100.0, 50.0, 200.0, 100.0), 0);
        assert!(display_point_to_texture(50.0, 75.0, &c).is_none()); // left of dest
        assert!(display_point_to_texture(150.0, 25.0, &c).is_none()); // above dest
        assert!(display_point_to_texture(350.0, 75.0, &c).is_none()); // right of dest
        assert!(display_point_to_texture(150.0, 200.0, &c).is_none()); // below dest
        assert!(display_point_to_texture(150.0, 100.0, &c).is_some()); // inside dest
    }

    #[test]
    fn point_transforms_apply_inverse_mapping() {
        let swapped = (
            (0.0, 0.0, 100.0, 200.0),
            (0.0, 0.0, 200.0, 100.0),
            (100.0, 200.0),
        );
        let hd = ((0.0, 0.0, 1920.0, 1080.0), (0.0, 0.0, 1920.0, 1080.0));
        let cases = [
            (swapped, 1, (0.0, 0.0), (0.0, 200.0)),
            (swapped, 1, (100.0, 0.0), (0.0, 0.0)),
            (swapped, 1, (100.0, 200.0), (100.0, 0.0)),
            (swapped, 1, (0.0, 200.0), (100.0, 200.0)),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                2,
                (0.0, 0.0),
                (1920.0, 1080.0),
            ),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                2,
                (1920.0, 1080.0),
                (0.0, 0.0),
            ),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                2,
                (960.0, 540.0),
                (960.0, 540.0),
            ),
            (swapped, 3, (0.0, 0.0), (100.0, 0.0)),
            (swapped, 3, (100.0, 0.0), (100.0, 200.0)),
            (swapped, 3, (100.0, 200.0), (0.0, 200.0)),
            (swapped, 3, (0.0, 200.0), (0.0, 0.0)),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                4,
                (0.0, 100.0),
                (1920.0, 100.0),
            ),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                4,
                (1920.0, 100.0),
                (0.0, 100.0),
            ),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                4,
                (960.0, 540.0),
                (960.0, 540.0),
            ),
            (swapped, 5, (0.0, 0.0), (0.0, 0.0)),
            (swapped, 5, (50.0, 100.0), (50.0, 100.0)),
            (swapped, 5, (100.0, 200.0), (100.0, 200.0)),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                6,
                (100.0, 0.0),
                (100.0, 1080.0),
            ),
            (
                (hd.0, hd.1, (1920.0, 1080.0)),
                6,
                (100.0, 1080.0),
                (100.0, 0.0),
            ),
            (swapped, 7, (0.0, 0.0), (100.0, 200.0)),
            (swapped, 7, (50.0, 100.0), (50.0, 100.0)),
            (swapped, 7, (100.0, 200.0), (0.0, 0.0)),
        ];

        for ((source, dest, display), transform, point, expected) in cases {
            let c = cfg_with_display(source, dest, display, transform);
            approx(
                display_point_to_texture(point.0, point.1, &c).unwrap(),
                expected,
            );
        }
    }

    #[test]
    fn point_cw90_maps_actual_portrait_display_domain() {
        let c = cfg_with_display(
            (0.0, 0.0, 1920.0, 1080.0),
            (0.0, 0.0, 1920.0, 1080.0),
            (1080.0, 1920.0),
            1,
        );
        approx(
            display_point_to_texture(540.0, 960.0, &c).unwrap(),
            (960.0, 540.0),
        );
        approx(
            display_point_to_texture(540.0, 1500.0, &c).unwrap(),
            (1500.0, 540.0),
        );
    }

    #[test]
    fn point_cw90_drops_rotated_letterbox_bar() {
        let c = cfg_with_display(
            (0.0, 0.0, 200.0, 100.0),
            (0.0, 25.0, 200.0, 50.0),
            (100.0, 200.0),
            1,
        );
        approx(
            display_point_to_texture(50.0, 100.0, &c).unwrap(),
            (100.0, 50.0),
        );
        assert!(display_point_to_texture(10.0, 100.0, &c).is_none());
        assert!(display_point_to_texture(90.0, 100.0, &c).is_none());
    }

    #[test]
    fn point_unknown_transform_falls_back_to_identity() {
        // Defensive: unknown transform shouldn't panic.
        let c = cfg((0.0, 0.0, 1920.0, 1080.0), (0.0, 0.0, 1920.0, 1080.0), 99);
        approx(
            display_point_to_texture(100.0, 50.0, &c).unwrap(),
            (100.0, 50.0),
        );
    }

    #[test]
    fn point_zero_dest_rect_returns_none() {
        let c = cfg((0.0, 0.0, 1920.0, 1080.0), (0.0, 0.0, 0.0, 0.0), 0);
        assert!(display_point_to_texture(0.0, 0.0, &c).is_none());
    }
}

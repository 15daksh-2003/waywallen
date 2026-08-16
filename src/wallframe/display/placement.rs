use serde::{Deserialize, Serialize};

use crate::settings::ResolvedLayout;
use crate::wallframe::display::layout::{self, LayoutInput, Rotation};
use crate::wallframe::routing::table::{LinkDstRect, LinkSrcRect};

pub const MAX_CANVAS_EXTENT: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl CanvasRect {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.width == 0 || self.height == 0 {
            return Err("canvas member must have a non-zero size");
        }
        if self.width > MAX_CANVAS_EXTENT || self.height > MAX_CANVAS_EXTENT {
            return Err("canvas member exceeds the supported extent");
        }
        if self.right().is_none() || self.bottom().is_none() {
            return Err("canvas member coordinates overflow");
        }
        Ok(self)
    }

    pub fn right(self) -> Option<i64> {
        i64::from(self.x).checked_add(i64::from(self.width))
    }

    pub fn bottom(self) -> Option<i64> {
        i64::from(self.y).checked_add(i64::from(self.height))
    }
}

pub fn union(rects: impl IntoIterator<Item = CanvasRect>) -> Option<CanvasRect> {
    let mut rects = rects.into_iter();
    let first = rects.next()?.validate().ok()?;
    let mut left = i64::from(first.x);
    let mut top = i64::from(first.y);
    let mut right = first.right()?;
    let mut bottom = first.bottom()?;
    for rect in rects {
        let rect = rect.validate().ok()?;
        left = left.min(i64::from(rect.x));
        top = top.min(i64::from(rect.y));
        right = right.max(rect.right()?);
        bottom = bottom.max(rect.bottom()?);
    }
    let width = u32::try_from(right.checked_sub(left)?).ok()?;
    let height = u32::try_from(bottom.checked_sub(top)?).ok()?;
    CanvasRect {
        x: i32::try_from(left).ok()?,
        y: i32::try_from(top).ok()?,
        width,
        height,
    }
    .validate()
    .ok()
}

pub fn canonicalize(rects: &mut [CanvasRect]) -> Result<Option<CanvasRect>, &'static str> {
    if rects.is_empty() {
        return Ok(None);
    }
    for rect in rects.iter().copied() {
        rect.validate()?;
    }
    let extent = union(rects.iter().copied()).ok_or("canvas extent is invalid")?;
    for rect in rects {
        rect.x = i32::try_from(i64::from(rect.x) - i64::from(extent.x))
            .map_err(|_| "canvas member x coordinate overflow")?;
        rect.y = i32::try_from(i64::from(rect.y) - i64::from(extent.y))
            .map_err(|_| "canvas member y coordinate overflow")?;
    }
    Ok(Some(CanvasRect {
        x: 0,
        y: 0,
        width: extent.width,
        height: extent.height,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasProjection {
    pub source: LinkSrcRect,
    pub dest: LinkDstRect,
    pub transform: u32,
}

pub fn project_canvas(
    texture_width: u32,
    texture_height: u32,
    surface_width: u32,
    surface_height: u32,
    canvas: CanvasRect,
    member: CanvasRect,
    layout: ResolvedLayout,
    clear_rgba: [f32; 4],
) -> Option<CanvasProjection> {
    canvas.validate().ok()?;
    member.validate().ok()?;
    if texture_width == 0 || texture_height == 0 || surface_width == 0 || surface_height == 0 {
        return None;
    }

    let member_x = f64::from(member.x) - f64::from(canvas.x);
    let member_y = f64::from(member.y) - f64::from(canvas.y);
    let canvas_width = f64::from(canvas.width);
    let canvas_height = f64::from(canvas.height);
    let member_width = f64::from(member.width);
    let member_height = f64::from(member.height);
    let (
        layout_width,
        layout_height,
        member_left,
        member_top,
        member_layout_width,
        member_layout_height,
    ) = match layout.rotation {
        Rotation::Normal => (
            canvas.width,
            canvas.height,
            member_x,
            member_y,
            member_width,
            member_height,
        ),
        Rotation::Cw90 => (
            canvas.height,
            canvas.width,
            member_y,
            canvas_width - member_x - member_width,
            member_height,
            member_width,
        ),
        Rotation::Cw180 => (
            canvas.width,
            canvas.height,
            canvas_width - member_x - member_width,
            canvas_height - member_y - member_height,
            member_width,
            member_height,
        ),
        Rotation::Cw270 => (
            canvas.height,
            canvas.width,
            canvas_height - member_y - member_height,
            member_x,
            member_height,
            member_width,
        ),
    };
    let (dest_surface_width, dest_surface_height) = match layout.rotation {
        Rotation::Cw90 | Rotation::Cw270 => (f64::from(surface_height), f64::from(surface_width)),
        _ => (f64::from(surface_width), f64::from(surface_height)),
    };
    let output = layout::compute(LayoutInput {
        tex_w: texture_width as f32,
        tex_h: texture_height as f32,
        disp_w: layout_width as f32,
        disp_h: layout_height as f32,
        fillmode: layout.fillmode,
        location: layout.location,
        clear_rgba,
    });

    let content_left = f64::from(output.dest.0);
    let content_top = f64::from(output.dest.1);
    let content_right = content_left + f64::from(output.dest.2);
    let content_bottom = content_top + f64::from(output.dest.3);
    let member_right = member_left + member_layout_width;
    let member_bottom = member_top + member_layout_height;
    let left = content_left.max(member_left);
    let top = content_top.max(member_top);
    let right = content_right.min(member_right);
    let bottom = content_bottom.min(member_bottom);
    if output.source.2 <= 0.0
        || output.source.3 <= 0.0
        || output.dest.2 <= 0.0
        || output.dest.3 <= 0.0
    {
        return None;
    }

    if right <= left || bottom <= top {
        return Some(CanvasProjection {
            source: LinkSrcRect {
                x: output.source.0,
                y: output.source.1,
                w: output.source.2,
                h: output.source.3,
            },
            dest: LinkDstRect {
                x: ((content_left - member_left) / member_layout_width * dest_surface_width) as f32,
                y: ((content_top - member_top) / member_layout_height * dest_surface_height) as f32,
                w: (f64::from(output.dest.2) / member_layout_width * dest_surface_width) as f32,
                h: (f64::from(output.dest.3) / member_layout_height * dest_surface_height) as f32,
            },
            transform: layout.rotation.to_wl_transform(),
        });
    }

    let source_x = f64::from(output.source.0)
        + (left - content_left) / f64::from(output.dest.2) * f64::from(output.source.2);
    let source_y = f64::from(output.source.1)
        + (top - content_top) / f64::from(output.dest.3) * f64::from(output.source.3);
    let source_w = (right - left) / f64::from(output.dest.2) * f64::from(output.source.2);
    let source_h = (bottom - top) / f64::from(output.dest.3) * f64::from(output.source.3);
    let dest_x = (left - member_left) / member_layout_width * dest_surface_width;
    let dest_y = (top - member_top) / member_layout_height * dest_surface_height;
    let dest_w = (right - left) / member_layout_width * dest_surface_width;
    let dest_h = (bottom - top) / member_layout_height * dest_surface_height;

    Some(CanvasProjection {
        source: LinkSrcRect {
            x: source_x as f32,
            y: source_y as f32,
            w: source_w as f32,
            h: source_h as f32,
        },
        dest: LinkDstRect {
            x: dest_x as f32,
            y: dest_y as f32,
            w: dest_w as f32,
            h: dest_h as f32,
        },
        transform: layout.rotation.to_wl_transform(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallframe::display::layout::{FillMode, Location};

    fn layout(fillmode: FillMode) -> ResolvedLayout {
        ResolvedLayout {
            fillmode,
            location: Location::default(),
            rotation: Rotation::Normal,
        }
    }

    #[test]
    fn union_preserves_negative_origin_and_gaps() {
        let canvas = union([
            CanvasRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
            CanvasRect {
                x: 200,
                y: 120,
                width: 2560,
                height: 1440,
            },
        ])
        .unwrap();
        assert_eq!(
            canvas,
            CanvasRect {
                x: -1920,
                y: 0,
                width: 4680,
                height: 1560
            }
        );
    }

    #[test]
    fn canonicalize_moves_the_union_origin_without_changing_relative_layout() {
        let mut members = [
            CanvasRect {
                x: -1920,
                y: 240,
                width: 1920,
                height: 1080,
            },
            CanvasRect {
                x: 320,
                y: -120,
                width: 2560,
                height: 1440,
            },
        ];
        let extent = canonicalize(&mut members).unwrap().unwrap();
        assert_eq!(extent.x, 0);
        assert_eq!(extent.y, 0);
        assert_eq!(extent.width, 4800);
        assert_eq!(extent.height, 1440);
        assert_eq!(members[0].x, 0);
        assert_eq!(members[0].y, 360);
        assert_eq!(members[1].x, 2240);
        assert_eq!(members[1].y, 0);
    }

    #[test]
    fn horizontal_span_slices_one_texture() {
        let canvas = CanvasRect {
            x: 0,
            y: 0,
            width: 3840,
            height: 1080,
        };
        let left = project_canvas(
            3840,
            1080,
            1920,
            1080,
            canvas,
            CanvasRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            layout(FillMode::Stretched),
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let right = project_canvas(
            3840,
            1080,
            2560,
            1440,
            canvas,
            CanvasRect {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
            layout(FillMode::Stretched),
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        assert_eq!(
            left.source,
            LinkSrcRect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0
            }
        );
        assert_eq!(
            right.source,
            LinkSrcRect {
                x: 1920.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0
            }
        );
        assert_eq!(
            right.dest,
            LinkDstRect {
                x: 0.0,
                y: 0.0,
                w: 2560.0,
                h: 1440.0
            }
        );
    }

    #[test]
    fn vertical_span_maps_logical_slice_to_each_surface_extent() {
        let canvas = CanvasRect {
            x: -200,
            y: -300,
            width: 100,
            height: 200,
        };
        let bottom = project_canvas(
            100,
            200,
            200,
            100,
            canvas,
            CanvasRect {
                x: -200,
                y: -200,
                width: 100,
                height: 100,
            },
            layout(FillMode::Stretched),
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        assert_eq!(
            bottom.source,
            LinkSrcRect {
                x: 0.0,
                y: 100.0,
                w: 100.0,
                h: 100.0
            }
        );
        assert_eq!(
            bottom.dest,
            LinkDstRect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 100.0
            }
        );

        let config = crate::wallframe::scheduler::CompositionConfig {
            generation: 1,
            buffer_generation: 1,
            display_w: 200.0,
            display_h: 100.0,
            source_x: bottom.source.x,
            source_y: bottom.source.y,
            source_w: bottom.source.w,
            source_h: bottom.source.h,
            dest_x: bottom.dest.x,
            dest_y: bottom.dest.y,
            dest_w: bottom.dest.w,
            dest_h: bottom.dest.h,
            transform: bottom.transform,
            clear_rgba: [0.0, 0.0, 0.0, 1.0],
        };
        assert_eq!(
            layout::display_point_to_texture(100.0, 50.0, &config),
            Some((50.0, 150.0))
        );
    }

    #[test]
    fn fit_places_non_intersecting_content_outside_the_member_surface() {
        let projection = project_canvas(
            100,
            100,
            100,
            100,
            CanvasRect {
                x: 0,
                y: 0,
                width: 300,
                height: 100,
            },
            CanvasRect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            layout(FillMode::PreserveAspectFit),
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        assert_eq!(
            projection.source,
            LinkSrcRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            }
        );
        assert_eq!(
            projection.dest,
            LinkDstRect {
                x: 100.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            }
        );
    }

    #[test]
    fn fit_places_rotated_non_intersecting_content_outside_the_member_surface() {
        let projection = project_canvas(
            100,
            100,
            100,
            100,
            CanvasRect {
                x: 0,
                y: 0,
                width: 300,
                height: 100,
            },
            CanvasRect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            ResolvedLayout {
                rotation: Rotation::Cw90,
                ..layout(FillMode::PreserveAspectFit)
            },
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        assert!(projection.source.w > 0.0);
        assert!(projection.source.h > 0.0);
        assert!(projection.dest.w > 0.0);
        assert!(projection.dest.h > 0.0);
        assert!(projection.dest.y + projection.dest.h <= 0.0);
        assert_eq!(projection.transform, Rotation::Cw90.to_wl_transform());
    }

    #[test]
    fn clockwise_rotation_partitions_the_group_before_surface_transform() {
        let canvas = CanvasRect {
            x: -100,
            y: 20,
            width: 200,
            height: 100,
        };
        let left = project_canvas(
            100,
            200,
            100,
            100,
            canvas,
            CanvasRect {
                x: -100,
                y: 20,
                width: 100,
                height: 100,
            },
            ResolvedLayout {
                rotation: Rotation::Cw90,
                ..layout(FillMode::Stretched)
            },
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let right = project_canvas(
            100,
            200,
            100,
            100,
            canvas,
            CanvasRect {
                x: 0,
                y: 20,
                width: 100,
                height: 100,
            },
            ResolvedLayout {
                rotation: Rotation::Cw90,
                ..layout(FillMode::Stretched)
            },
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        assert_eq!(
            left.source,
            LinkSrcRect {
                x: 0.0,
                y: 100.0,
                w: 100.0,
                h: 100.0
            }
        );
        assert_eq!(
            right.source,
            LinkSrcRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0
            }
        );
        assert_eq!(
            left.dest,
            LinkDstRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0
            }
        );
        assert_eq!(left.transform, Rotation::Cw90.to_wl_transform());
    }
}

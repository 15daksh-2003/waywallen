use super::*;

pub(super) fn project_link(
    link: &Link,
    pool: &PublishedPool,
    info: &DisplayInfo,
    config_generation: u64,
    buffer_generation: u64,
    layout: &ResolvedLayout,
) -> CompositionConfig {
    if let crate::wallframe::routing::table::LinkProjection::Canvas {
        extent,
        member,
        layout: canvas_layout,
        ..
    } = &link.projection
    {
        let projection = crate::wallframe::display::placement::project_canvas(
            pool.width,
            pool.height,
            info.metrics.width,
            info.metrics.height,
            *extent,
            *member,
            *canvas_layout,
            link.clear_rgba,
        );
        return match projection {
            Some(projection) => CompositionConfig {
                generation: config_generation,
                buffer_generation,
                display_w: info.metrics.width as f32,
                display_h: info.metrics.height as f32,
                source_x: projection.source.x,
                source_y: projection.source.y,
                source_w: projection.source.w,
                source_h: projection.source.h,
                dest_x: projection.dest.x,
                dest_y: projection.dest.y,
                dest_w: projection.dest.w,
                dest_h: projection.dest.h,
                transform: projection.transform,
                clear_rgba: link.clear_rgba,
            },
            None => CompositionConfig {
                generation: config_generation,
                buffer_generation,
                display_w: info.metrics.width as f32,
                display_h: info.metrics.height as f32,
                source_x: 0.0,
                source_y: 0.0,
                source_w: 0.0,
                source_h: 0.0,
                dest_x: 0.0,
                dest_y: 0.0,
                dest_w: 0.0,
                dest_h: 0.0,
                transform: canvas_layout.rotation.to_wl_transform(),
                clear_rgba: link.clear_rgba,
            },
        };
    }

    let src_full = link.src_rect == crate::wallframe::routing::table::FULL_SRC;
    let dst_full = link.dst_rect == crate::wallframe::routing::table::FULL_DST;

    if src_full && dst_full {
        let (tex_w, tex_h) = (pool.width, pool.height);
        let (eff_disp_w, eff_disp_h) = match layout.rotation {
            crate::wallframe::display::layout::Rotation::Cw90
            | crate::wallframe::display::layout::Rotation::Cw270 => {
                (info.metrics.height as f32, info.metrics.width as f32)
            }
            _ => (info.metrics.width as f32, info.metrics.height as f32),
        };
        let out = crate::wallframe::display::layout::compute(LayoutInput {
            tex_w: tex_w as f32,
            tex_h: tex_h as f32,
            disp_w: eff_disp_w,
            disp_h: eff_disp_h,
            fillmode: layout.fillmode,
            location: layout.location,
            clear_rgba: link.clear_rgba,
        });
        return CompositionConfig {
            generation: config_generation,
            buffer_generation,
            display_w: info.metrics.width as f32,
            display_h: info.metrics.height as f32,
            source_x: out.source.0,
            source_y: out.source.1,
            source_w: out.source.2,
            source_h: out.source.3,
            dest_x: out.dest.0,
            dest_y: out.dest.1,
            dest_w: out.dest.2,
            dest_h: out.dest.3,
            transform: layout.rotation.to_wl_transform(),
            clear_rgba: out.clear_rgba,
        };
    }

    let (texture_width, texture_height) = (pool.width, pool.height);
    let resolve_src = |rect: LinkSrcRect| -> (f32, f32, f32, f32) {
        let width = if rect.w.is_infinite() {
            texture_width as f32
        } else {
            rect.w
        };
        let height = if rect.h.is_infinite() {
            texture_height as f32
        } else {
            rect.h
        };
        (rect.x, rect.y, width, height)
    };
    let resolve_dst = |rect: LinkDstRect| -> (f32, f32, f32, f32) {
        let width = if rect.w.is_infinite() {
            info.metrics.width as f32
        } else {
            rect.w
        };
        let height = if rect.h.is_infinite() {
            info.metrics.height as f32
        } else {
            rect.h
        };
        (rect.x, rect.y, width, height)
    };
    let (source_x, source_y, source_w, source_h) = resolve_src(link.src_rect);
    let (dest_x, dest_y, dest_w, dest_h) = resolve_dst(link.dst_rect);
    CompositionConfig {
        generation: config_generation,
        buffer_generation,
        display_w: info.metrics.width as f32,
        display_h: info.metrics.height as f32,
        source_x,
        source_y,
        source_w,
        source_h,
        dest_x,
        dest_y,
        dest_w,
        dest_h,
        transform: layout.rotation.to_wl_transform(),
        clear_rgba: link.clear_rgba,
    }
}

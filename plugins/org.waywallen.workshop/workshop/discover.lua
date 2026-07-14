local api = import("workshop.api")
local map = import("workshop.map")
local project = import("workshop.project")

local M = {}

function M.tags(ctx)
    return api.fetch_tags(ctx)
end

function M.search(ctx, params)
    local result = api.search(ctx, params)
    local items = {}
    for _, item in ipairs(result.items) do
        table.insert(items, map.search_item(item))
    end
    local page = params.page or 1
    return {
        items = items,
        has_more = page * result.numperpage < result.total,
    }
end

function M.details(ctx, id)
    return map.details(api.details(ctx, id))
end

function M.download(ctx, id)
    return map.download(api.details(ctx, id))
end

function M.resolve(ctx, params)
    local dir = params.dir
    local raw = ctx.read_file(dir .. "/project.json")
    if not raw then
        error("project.json not found in downloaded item")
    end
    local proj = ctx.json_parse(raw)
    if not proj then
        error("project.json is not valid JSON")
    end
    local project_type = proj.type and string.lower(proj.type) or ""
    local wp_type, resource = project.classify(ctx, dir, proj, project_type)
    if not wp_type then
        error("unsupported or empty Wallpaper Engine item")
    end
    local preview = project.pick_preview(ctx, dir, proj)
    return {
        name = proj.title or params.id,
        wp_type = wp_type,
        resource = project.relpath(dir, resource),
        preview = project.relpath(dir, preview),
        description = proj.description or "",
        tags = proj.tags or {},
        external_id = params.id,
        content_rating = proj.contentrating,
    }
end

return M

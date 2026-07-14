-- Mostly ported from open-wallpaper-engine wallpaper_engine/project.lua
-- https://github.com/waywallen/open-wallpaper-engine/blob/main/waywallen/plugins/org.waywallen.open-wallpaper-engine/wallpaper_engine/project.lua

local M = {}

local VIDEO_EXTS = { mp4 = true, webm = true, mkv = true, avi = true, mov = true }

function M.pick_preview(ctx, dir, project)
    if project and project.preview then
        local p = dir .. "/" .. project.preview
        if ctx.file_exists(p) then return p end
    end
    for _, p in ipairs({ dir .. "/preview.jpg", dir .. "/preview.png", dir .. "/preview.gif" }) do
        if ctx.file_exists(p) then return p end
    end
    return nil
end

function M.classify(ctx, dir, project, project_type)
    if project_type == "web" then
        if ctx.file_exists(dir .. "/project.json") then
            return "web", dir
        end
    elseif project_type == "video" then
        local file = project and project.file
        if file and ctx.file_exists(dir .. "/" .. file) then
            return "video", dir .. "/" .. file
        end
        for _, path in ipairs(ctx.glob(dir .. "/*.*")) do
            local ext = ctx.extension(path)
            if ext and VIDEO_EXTS[string.lower(ext)] then
                return "video", path
            end
        end
    else
        if ctx.file_exists(dir .. "/scene.pkg") then
            return "scene", dir .. "/scene.pkg"
        elseif ctx.file_exists(dir .. "/scene.json") then
            return "scene", dir .. "/scene.json"
        end
    end
    return nil, nil
end

-- Strip the item-directory prefix so resource/preview are relative to it, the
-- form the daemon's resolve upsert expects.
function M.relpath(dir, abs)
    if not abs then
        return nil
    end
    local d = dir:gsub("/+$", "")
    if abs == d then
        return ""
    end
    local prefix = d .. "/"
    if abs:sub(1, #prefix) == prefix then
        return abs:sub(#prefix + 1)
    end
    return abs
end

return M

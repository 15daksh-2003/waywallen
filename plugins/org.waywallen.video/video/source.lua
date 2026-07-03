local M = {}

local VIDEO_EXTS = {
    mp4 = true, m4v = true, mkv = true, webm = true,
    mov = true, avi = true, flv = true, wmv = true,
    mpg = true, mpeg = true, ts = true, m2ts = true,
    ogv = true, ogm = true,
}

local function strip_ext(name)
    return name:match("(.+)%.[^.]+$") or name
end

local function first_existing(ctx, candidates)
    local out, seen = {}, {}
    for _, p in ipairs(candidates) do
        if p and p ~= "" and not seen[p] and ctx.file_exists(p) then
            seen[p] = true
            table.insert(out, p)
        end
    end
    return out
end

function M.auto_detect(ctx)
    local home = ctx.env("HOME")
    local videos = ctx.env("XDG_VIDEOS_DIR")
    local pictures = ctx.env("XDG_PICTURES_DIR")

    local candidates = {}
    if videos and videos ~= "" then table.insert(candidates, videos) end
    if home and home ~= "" then table.insert(candidates, home .. "/Videos/Wallpapers") end
    if home and home ~= "" then table.insert(candidates, home .. "/Videos") end
    if pictures and pictures ~= "" then table.insert(candidates, pictures .. "/Wallpapers") end
    if home and home ~= "" then table.insert(candidates, home .. "/Pictures/Wallpapers") end
    return first_existing(ctx, candidates)
end

function M.remove(ctx, item)
    ctx.remove_file(item.path or item.resource)
end

function M.scan(ctx)
    local entries = {}
    local dirs = {}
    for _, d in ipairs(ctx.libraries()) do
        if ctx.file_exists(d) then table.insert(dirs, d) end
    end
    if #dirs == 0 then
        ctx.log("video: no video libraries configured")
        return entries
    end

    local seen_path = {}
    for _, dir in ipairs(dirs) do
        local patterns = {
            dir .. "/*.*",
            dir .. "/*/*.*",
            dir .. "/*/*/*.*",
        }
        for _, pat in ipairs(patterns) do
            for _, path in ipairs(ctx.glob(pat)) do
                local ext = ctx.extension(path)
                if ext and VIDEO_EXTS[string.lower(ext)] and not seen_path[path] then
                    seen_path[path] = true
                    local filename = ctx.filename(path) or path
                    table.insert(entries, {
                        name = strip_ext(filename),
                        wp_type = "video",
                        resource = path,
                        preview = nil,
                        library_root = dir,
                        size = ctx.file_size(path),
                    })
                end
            end
        end
    end

    ctx.log("video: found " .. #entries .. " video wallpapers in "
            .. #dirs .. " directories")
    return entries
end

return M

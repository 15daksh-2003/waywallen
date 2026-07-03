local M = {}

local IMAGE_EXTS = {
    png = true, jpg = true, jpeg = true, webp = true,
    bmp = true, tiff = true, tif = true, avif = true, gif = true,
}

local function strip_ext(name)
    return name:match("(.+)%.[^.]+$") or name
end

function M.auto_detect(ctx)
    local candidates = {}
    local xdg = ctx.env("XDG_PICTURES_DIR")
    if xdg and xdg ~= "" then table.insert(candidates, xdg) end
    local home = ctx.env("HOME")
    if home and home ~= "" then table.insert(candidates, home .. "/Pictures/Wallpapers") end

    local found, seen = {}, {}
    for _, p in ipairs(candidates) do
        if not seen[p] and ctx.file_exists(p) then
            seen[p] = true
            table.insert(found, p)
        end
    end
    return found
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
        ctx.log("image: no image libraries configured")
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
                if ext and IMAGE_EXTS[string.lower(ext)] and not seen_path[path] then
                    seen_path[path] = true
                    local filename = ctx.filename(path) or path
                    table.insert(entries, {
                        name = strip_ext(filename),
                        wp_type = "image",
                        resource = path,
                        library_root = dir,
                    })
                end
            end
        end
    end

    ctx.log("image: found " .. #entries .. " image wallpapers in "
            .. #dirs .. " directories")
    return entries
end

return M

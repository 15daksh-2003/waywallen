local M = {}

local ASSETS_REL = "/steamapps/common/wallpaper_engine/assets"

local function data_home(ctx)
    local xdg = ctx.env("XDG_DATA_HOME")
    if xdg and xdg ~= "" then
        return xdg
    end
    local home = ctx.env("HOME")
    if home and home ~= "" then
        return home .. "/.local/share"
    end
    return nil
end

local function we_assets(ctx)
    local configured = ctx.plugin_config and ctx.plugin_config("wallpaper_engine_assets")
    if configured and configured ~= "" and ctx.file_exists(configured) then
        return configured
    end
    local data = data_home(ctx)
    if data then
        local managed = data .. "/waywallen/wallpaper_engine/assets"
        if ctx.file_exists(managed) then
            return managed
        end
    end
    local home = ctx.env("HOME") or ""
    local roots = {
        home .. "/.local/share/Steam",
        home .. "/.steam/steam",
        home .. "/.steam/root",
        home .. "/.var/app/com.valvesoftware.Steam/data/Steam",
    }
    for _, root in ipairs(roots) do
        local p = root .. ASSETS_REL
        if ctx.file_exists(p) then
            return p
        end
    end
    return nil
end

function M.extras(entry, ctx)
    local out = { path = entry.resource }
    if entry.wp_type == "scene" or entry.wp_type == "web" then
        local assets = we_assets(ctx)
        if assets then
            out.assets = assets
        end
    end
    if entry.external_id and entry.external_id ~= "" then
        out.workshop_id = entry.external_id
    end
    return out
end

return M

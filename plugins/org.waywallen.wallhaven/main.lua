local discover = import("wallhaven.discover")
local api = import("wallhaven.api")
local source = import("wallhaven.source")
local wallpaper = import("wallhaven.wallpaper")

local M = {}

function M.info()
    return {
        name = "wallhaven",
        capabilities = {
            discover = {
                search = true,
                details = true,
                download = true,
                sorts = {
                    { key = "trend", label = "Trending" },
                    { key = "recent", label = "Recent" },
                    { key = "popular", label = "Popular" },
                },
                tags = api.tags,
            },
            wallpaper = {
                extras = true,
            },
        },
    }
end

M.discover = discover
M.source = source
M.wallpaper = wallpaper

return M

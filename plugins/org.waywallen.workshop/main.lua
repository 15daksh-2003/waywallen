local discover = import("workshop.discover")
local source = import("workshop.source")
local wallpaper = import("workshop.wallpaper")
local api = import("workshop.api")

local M = {}

function M.info()
    return {
        name = "workshop",
        display_name = "Steam Workshop",
        status = {
            { id = "steam_account", label = "Status", group = "Steam account", order = 20 },
        },
        actions = {
            { id = "steam_sign_in", label = "Sign in to Steam", group = "Steam account", order = 21 },
            { id = "steam_sign_out", label = "Sign out", group = "Steam account", order = 22 },
        },
        capabilities = {
            discover = {
                search = true,
                details = true,
                download = true,
                resolve = true,
                sorts = {
                    { key = "trend_day", label = "Trending today" },
                    { key = "trend_week", label = "Trending this week" },
                    { key = "trend_month", label = "Trending this month" },
                    { key = "trend_3months", label = "Trending 3 months" },
                    { key = "trend_6months", label = "Trending 6 months" },
                    { key = "trend_year", label = "Trending this year" },
                    { key = "recent", label = "Most recent" },
                    { key = "most_subscribed", label = "Most subscribed" },
                    { key = "top_rated", label = "Top rated" },
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

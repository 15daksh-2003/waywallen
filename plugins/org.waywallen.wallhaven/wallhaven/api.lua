local M = {}
local session = import("wallhaven.session")

local API = "https://wallhaven.cc/api/v1"

local SORTS = {
    trend = { sorting = "toplist", topRange = "1M" },
    recent = { sorting = "date_added" },
    popular = { sorting = "favorites" },
}

local RESOLUTION_TAGS = {
    ["1280 x 720"] = "1280x720",
    ["1920 x 1080"] = "1920x1080",
    ["2560 x 1440"] = "2560x1440",
    ["3840 x 2160"] = "3840x2160",
    ["2560 x 1080"] = "2560x1080",
    ["3440 x 1440"] = "3440x1440",
    ["5120 x 1440"] = "5120x1440",
    ["3840 x 1080"] = "3840x1080",
    ["7680 x 2160"] = "7680x2160",
    ["1080 x 1920"] = "1080x1920",
    ["720 x 1280"] = "720x1280",
    ["1440 x 2560"] = "1440x2560",
}

local QUERY_TAGS = {
    Abstract = "abstract",
    Animal = "animal",
    Anime = "anime",
    Cartoon = "cartoon",
    CG = "cg",
    Cyberpunk = "cyberpunk",
    Fantasy = "fantasy",
    Game = "game",
    Girls = "girl",
    Guys = "guy",
    Landscape = "landscape",
    Medieval = "medieval",
    Memes = "meme",
    MMD = "mmd",
    Music = "music",
    Nature = "nature",
    ["Pixel art"] = "pixel art",
    Relaxing = "relaxing",
    Retro = "retro",
    ["Sci-Fi"] = "sci-fi",
    Sports = "sports",
    Technology = "technology",
    Vehicle = "vehicle",
}

M.filters = {
    {
        id = "purity",
        title = "Purity",
        type = "multi_select",
        description = "NSFW wallpapers require a Wallhaven API key.",
        values = {
            "SFW",
            "Sketchy",
            "NSFW",
        },
    },
    {
        id = "topics",
        title = "Topics",
        type = "multi_select",
        values = {
            "Abstract",
            "Animal",
            "Anime",
            "Cartoon",
            "CG",
            "Cyberpunk",
            "Fantasy",
            "Game",
            "Girls",
            "Guys",
            "Landscape",
            "Medieval",
            "Memes",
            "MMD",
            "Music",
            "Nature",
            "Pixel art",
            "Relaxing",
            "Retro",
            "Sci-Fi",
            "Sports",
            "Technology",
            "Vehicle",
        },
    },
    {
        id = "resolution",
        title = "Resolution",
        type = "multi_select",
        values = {
            "1280 x 720",
            "1920 x 1080",
            "2560 x 1440",
            "3840 x 2160",
            "2560 x 1080",
            "3440 x 1440",
            "5120 x 1440",
            "3840 x 1080",
            "7680 x 2160",
            "1080 x 1920",
            "720 x 1280",
            "1440 x 2560",
        },
    },
}

local function append_query(query, term)
    if not term or term == "" then
        return query
    end
    if query == "" then
        return term
    end
    return query .. " " .. term
end

local function request_json(ctx, url, query)
    local rsp = ctx.http:get(url)
        :headers(session.headers())
        :query(query)
        :timeout(20)
        :send()
    if rsp:status() == 401 or rsp:status() == 403 then
        session.sign_out()
        error("Wallhaven API key was rejected; log in again")
    end
    if not rsp:ok() then
        error("wallhaven http " .. tostring(rsp:status()))
    end
    return rsp:json()
end

function M.search(ctx, params)
    local sort = SORTS[params.sort] or SORTS.trend
    local query = params.query or ""
    local resolutions = {}
    local general, anime, people = true, true, true
    local category_selected = false
    local sfw, sketchy, nsfw = false, false, false
    local purity_selected = false

    for _, tag in ipairs(params.tags or {}) do
        if tag == "SFW" then
            sfw = true
            purity_selected = true
        elseif tag == "Sketchy" then
            sketchy = true
            purity_selected = true
        elseif tag == "NSFW" then
            nsfw = true
            purity_selected = true
        elseif tag == "Anime" then
            if not category_selected then
                general, anime, people = false, false, false
                category_selected = true
            end
            anime = true
        elseif tag == "Girls" or tag == "Guys" then
            if not category_selected then
                general, anime, people = false, false, false
                category_selected = true
            end
            people = true
        end
        local resolution = RESOLUTION_TAGS[tag]
        if resolution then
            table.insert(resolutions, resolution)
        end
        query = append_query(query, QUERY_TAGS[tag])
    end

    if not purity_selected then
        sfw = true
    end
    if nsfw and not session.authenticated() then
        error("Log in to Wallhaven with an API key to browse NSFW wallpapers")
    end

    local q = {
        q = query,
        categories = (general and "1" or "0") .. (anime and "1" or "0") .. (people and "1" or "0"),
        purity = (sfw and "1" or "0") .. (sketchy and "1" or "0") .. (nsfw and "1" or "0"),
        sorting = sort.sorting,
        order = "desc",
        page = tostring(params.page or 1),
    }
    if sort.topRange then
        q.topRange = sort.topRange
    end
    if #resolutions > 0 then
        q.resolutions = table.concat(resolutions, ",")
    end

    return request_json(ctx, API .. "/search", q)
end

function M.wallpaper(ctx, id)
    local payload = request_json(ctx, API .. "/w/" .. tostring(id), {})
    return payload.data or {}
end

return M

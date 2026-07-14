local M = {}

M.APPID = "431960"
local QUERYFILES = "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/"
local FILEDETAILS = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"
local BROWSE_PAGE = "https://steamcommunity.com/workshop/browse/?appid=" .. M.APPID
local NUMPERPAGE = 30

-- sort key -> EPublishedFileQueryType (+ trend window in days).
-- 0 RankedByVote, 1 RankedByPublicationDate, 3 RankedByTrend, 9 RankedByTotalUniqueSubscriptions.
local SORTS = {
    trend_day = { query_type = 3, days = 1 },
    trend_week = { query_type = 3, days = 7 },
    trend_month = { query_type = 3, days = 30 },
    trend_3months = { query_type = 3, days = 90 },
    trend_6months = { query_type = 3, days = 180 },
    trend_year = { query_type = 3, days = 365 },
    recent = { query_type = 1 },
    most_subscribed = { query_type = 9 },
    top_rated = { query_type = 0 },
}

-- Workshop type tags -> waywallen wp_type. Application is omitted (unrenderable, unless wine..?).
M.TYPE_WP = {
    Scene = "scene",
    Video = "video",
    Web = "web",
}

local RATING_TAGS = { Questionable = true, Mature = true }

-- Tags the workshop declares that never make sense as wallpaper filters: the
-- non-renderable/non-wallpaper types and categories, and the default rating.
local EXCLUDE_TAGS = {
    Application = true, Wallpaper = true, Preset = true, Asset = true,
    Everyone = true, ["Asset Pack"] = true,
}

-- Fallback
M.tags = { "Scene", "Video", "Web", "Questionable", "Mature" }

-- Web scraped tags
local cached_tags

function M.fetch_tags(ctx)
    if cached_tags then
        return cached_tags
    end
    local rsp = ctx.http:get(BROWSE_PAGE):timeout(15):send()
    if not rsp:ok() then
        error("steam workshop tags http " .. tostring(rsp:status()))
    end
    local html = rsp:text() or ""
    local d = html:find("declaredTags", 1, true)
    local s = d and html:find("mtx_tags", d, true)
    local e = s and html:find("readytouse_tags", s, true)
    if not s or not e then
        error("could not locate the workshop tag taxonomy")
    end
    local block = html:sub(s, e):gsub('\\"', '"')
    local tags, seen = {}, {}
    for name, admin in block:gmatch('"name":"([^"]+)","display_name":"[^"]*","admin_only":(%a+)') do
        if admin == "false" and not EXCLUDE_TAGS[name] and not seen[name] then
            seen[name] = true
            table.insert(tags, name)
        end
    end
    cached_tags = tags
    return tags
end

local function access_token(ctx)
    local token = ctx.steam_access_token and ctx.steam_access_token()
    if not token or token == "" then
        error(
            "Sign in to Steam to browse the Workshop. Open the Steam Workshop "
            .. "settings and use Sign in to Steam."
        )
    end
    return token
end

function M.search(ctx, params)
    local sort = SORTS[params.sort] or SORTS.trend_week

    -- Only wallpapers: never return applications, presets or asset packs.
    -- Ratings are excluded unless the user opted them in via a tag.
    local excluded = {
        Questionable = true, Mature = true,
        Application = true, Preset = true, Asset = true,
    }
    local required = {}
    for _, tag in ipairs(params.tags or {}) do
        if RATING_TAGS[tag] then
            excluded[tag] = nil
        else
            table.insert(required, tag)
        end
    end

    local query = {
        access_token = access_token(ctx),
        appid = M.APPID,
        query_type = tostring(sort.query_type),
        page = tostring(params.page or 1),
        numperpage = tostring(NUMPERPAGE),
        search_text = params.query or "",
        return_tags = "true",
        return_previews = "true",
        return_vote_data = "true",
        return_short_description = "true",
        format = "json",
    }
    if sort.days then
        query.days = tostring(sort.days)
    end
    for i, tag in ipairs(required) do
        query["requiredtags[" .. (i - 1) .. "]"] = tag
    end
    local ei = 0
    for tag in pairs(excluded) do
        query["excludedtags[" .. ei .. "]"] = tag
        ei = ei + 1
    end

    local rsp = ctx.http:get(QUERYFILES):query(query):timeout(20):send()
    if not rsp:ok() then
        error("steam workshop http " .. tostring(rsp:status()))
    end
    local body = rsp:json() or {}
    local response = body.response or {}
    return {
        items = response.publishedfiledetails or {},
        total = response.total or 0,
        numperpage = NUMPERPAGE,
    }
end

function M.details(ctx, id)
    local rsp = ctx.http
        :post(FILEDETAILS)
        :form({ itemcount = "1", ["publishedfileids[0]"] = tostring(id) })
        :timeout(20)
        :send()
    if not rsp:ok() then
        error("steam workshop http " .. tostring(rsp:status()))
    end
    local body = rsp:json() or {}
    local list = (body.response or {}).publishedfiledetails or {}
    return list[1] or {}
end

return M

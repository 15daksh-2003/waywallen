local M = {}

local FORMAT_VERSION = "wallhaven-session-v1"
local SETTINGS = "https://wallhaven.cc/api/v1/settings"

local api_key = ""
local last_check = { state = "signed_out", display_value = "Guest mode" }

local function remember(check)
    last_check = check
    return check
end

function M.load(blob)
    api_key = ""
    last_check = { state = "signed_out", display_value = "Guest mode" }
    if type(blob) ~= "string" or blob == "" then return end
    local prefix = FORMAT_VERSION .. "\n"
    if blob:sub(1, #prefix) ~= prefix then return end
    local separator = blob:find("\n", #prefix + 1, true)
    if not separator then return end
    local length = tonumber(blob:sub(#prefix + 1, separator - 1))
    local value = blob:sub(separator + 1)
    if not length or length ~= #value then return end
    api_key = value
end

function M.save()
    if api_key == "" then return "" end
    return FORMAT_VERSION .. "\n" .. tostring(#api_key) .. "\n" .. api_key
end

function M.check()
    if api_key == "" then
        return remember({ state = "signed_out", display_value = "Guest mode" })
    end
    return remember({ state = "signed_in", display_value = "API key active" })
end

function M.current_check()
    return last_check
end

function M.headers()
    if api_key == "" then return {} end
    return { ["X-API-Key"] = api_key }
end

function M.authenticated()
    return api_key ~= ""
end

function M.sign_in(ctx, value)
    local candidate = type(value) == "string" and value:match("^%s*(.-)%s*$") or ""
    if candidate == "" then error("Enter a Wallhaven API key") end
    local response = ctx.http:get(SETTINGS)
        :headers({ ["X-API-Key"] = candidate })
        :timeout(20)
        :send()
    if response:status() == 401 or response:status() == 403 then
        error("Wallhaven rejected this API key")
    end
    if not response:ok() then
        error("Wallhaven API key verification failed with HTTP " .. tostring(response:status()))
    end
    api_key = candidate
    remember({ state = "signed_in", display_value = "API key active" })
end

function M.sign_out()
    api_key = ""
    remember({ state = "signed_out", display_value = "Guest mode" })
end

return M

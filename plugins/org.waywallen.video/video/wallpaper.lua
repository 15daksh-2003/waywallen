local M = {}

function M.properties()
    return {
        ["waywallen.scheme_color"] = {
            text = "Scheme color",
            type = "color",
            value = {0.0, 0.0, 0.0, 1.0},
        },
        ["waywallen.enable_audio"] = {
            text = "Enable audio",
            type = "bool",
            value = true,
        },
        ["waywallen.playback_speed"] = {
            text = "Playback speed",
            type = "slider",
            min = 10,
            max = 400,
            step = 10,
            suffix = "%",
            value = 100,
        },
    }
end

function M.apply(entry)
    return {
        extras = {
            path = entry.resource,
        },
        default_user_properties = {},
    }
end

return M

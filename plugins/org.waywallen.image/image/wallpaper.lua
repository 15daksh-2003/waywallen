local M = {}

function M.properties()
    return {
        ["waywallen.scheme_color"] = {
            text = "Scheme color",
            type = "color",
            value = {0.0, 0.0, 0.0, 1.0},
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

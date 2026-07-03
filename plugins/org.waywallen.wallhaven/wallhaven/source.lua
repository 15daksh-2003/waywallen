local M = {}

function M.remove(ctx, item)
    local path = item.path or item.resource
    if not path or path == "" then
        error("missing item path")
    end
    ctx.remove_file(path)
    ctx.remove_file(path .. ".json")
end

return M

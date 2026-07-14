local M = {}

function M.remove(ctx, item)
    local root = item.library_root
    local id = item.external_id
    if not root or root == "" or not id or id == "" then
        error("missing item library_root/external_id")
    end
    ctx.remove_dir(root .. "/" .. id)
end

return M

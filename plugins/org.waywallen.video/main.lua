local source = import("video.source")
local wallpaper = import("video.wallpaper")

local M = {}

function M.info()
    return {
        name = "video",
        capabilities = {
            source = {
                types = {"video"},
                scan = true,
                auto_detect = true,
                library_label = "Video Folder",
                library_hint =
                    "A directory containing video files. Subdirectories up to two " ..
                    "levels deep are also scanned. Supported formats: MP4, MKV, " ..
                    "WebM, MOV, AVI, and other common containers.",
            },
            wallpaper = {
                properties = true,
                apply = true,
            },
        },
    }
end

M.source = source
M.wallpaper = wallpaper

return M

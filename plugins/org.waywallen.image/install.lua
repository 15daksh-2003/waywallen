local plugin = "share/waywallen/plugins/org.waywallen.image"

local function plugin_path(path)
  return plugin .. "/" .. path
end

lito.install({
  artifacts = {
    {
      target = { kind = "bin", name = "waywallen-image-renderer" },
      destination = "bin/waywallen-image-renderer",
    },
  },
  files = {
    { source = "plugin.toml", destination = plugin_path("plugin.toml") },
    { source = "main.lua", destination = plugin_path("main.lua") },
    { source = "image/source.lua", destination = plugin_path("image/source.lua") },
    { source = "image/wallpaper.lua", destination = plugin_path("image/wallpaper.lua") },
  },
  inventories = {
    {
      destination = plugin_path("files.txt"),
      relative_to = plugin,
    },
  },
})

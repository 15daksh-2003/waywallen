pragma ValueTypeBehavior: Assertable
import QtQuick
import Qcm.Material as MD
import waywallen.ui as W

// Compact colored chip identifying which GPU a renderer or display is on.
// Caller passes a DRM render-node id as (drmRenderMajor, drmRenderMinor);
// this resolves it via App.gpuManager and picks the matching vendor scheme.
// Stays invisible until the GpuList fetch completes or when major == 0.
Tag {
    id: root

    property int drmRenderMajor: 0
    property int drmRenderMinor: 0

    readonly property var gpu: (root.drmRenderMajor > 0 && W.App.gpuManager)
        ? W.App.gpuManager.find(root.drmRenderMajor, root.drmRenderMinor)
        : null

    readonly property var palette: root.gpu ? W.Global.gpu.forVendor(root.gpu.vendorId) : null

    text: {
        if (!root.gpu) return "";
        if (root.gpu.driver) return root.gpu.driver;
        return "drm:" + root.drmRenderMajor + ":" + root.drmRenderMinor;
    }

    visible: root.gpu !== null
    bgColor: root.palette ? root.palette.primary_container : MD.Token.color.surface_container_high
    fgColor: root.palette ? root.palette.on_primary_container : MD.Token.color.on_surface

    HoverHandler {
        id: hover
    }

    MD.ToolTip.visible: hover.hovered && root.gpu !== null
    MD.ToolTip.delay: 300
    MD.ToolTip.text: W.GpuDetails.toolTipText(root.gpu)
}

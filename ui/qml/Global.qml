pragma Singleton
import QtCore
import QtQuick
import Qcm.Material as MD
import waywallen.ui as W

// App-wide singleton state and derived theming.
QtObject {
    id: root

    property bool sidebarAutoExpand: true
    property int networkCacheMaximumMiB: 1024

    onNetworkCacheMaximumMiBChanged:
        W.App.setNetworkCacheMaximumSize(networkCacheMaximumMiB * 1024 * 1024)

    Component.onCompleted:
        W.App.setNetworkCacheMaximumSize(networkCacheMaximumMiB * 1024 * 1024)

    readonly property Component errorToastAction: Component {
        MD.Action {
            required property string error
            text: qsTr("Copy")
            onTriggered: {
                W.Action.copyToClipboard(error);
                W.Action.toast(qsTr("Copied to clipboard"), 2000);
            }
        }
    }

    function toastError(error) {
        const action = errorToastAction.createObject(root, {
            error: error
        });
        W.Action.toast(error, 0, MD.Enum.TFCloseable, action);
    }

    readonly property Settings _generalSettings: Settings {
        property alias sidebarAutoExpand: root.sidebarAutoExpand
        property alias networkCacheMaximumMiB: root.networkCacheMaximumMiB
    }

    // Per-vendor Material color schemes, seeded from each GPU vendor's brand
    // color and tracking the app theme mode, so vendor chips stay legible in
    // light and dark.
    readonly property QtObject gpu: QtObject {
        // PCI vendor IDs: AMD 0x1002, NVIDIA 0x10de, Intel 0x8086.
        readonly property MD.MdColorMgr amd: MD.MdColorMgr {
            accentColor: Qt.rgba(0.86, 0.20, 0.20, 1.0)
            mode: MD.Token.color.mode
        }
        readonly property MD.MdColorMgr nvidia: MD.MdColorMgr {
            accentColor: Qt.rgba(0.27, 0.66, 0.20, 1.0)
            mode: MD.Token.color.mode
        }
        readonly property MD.MdColorMgr intel: MD.MdColorMgr {
            accentColor: Qt.rgba(0.20, 0.45, 0.85, 1.0)
            mode: MD.Token.color.mode
        }

        function forVendor(vendorId) {
            if (vendorId === 0x1002)
                return amd;
            if (vendorId === 0x10de)
                return nvidia;
            if (vendorId === 0x8086)
                return intel;
            return null;
        }
    }
}

pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD

MD.BottomSheet {
    id: control

    required property Item popupParent
    required property var relay
    property var currentWallpaperSelect: null

    parent: popupParent
    anchors.fill: parent
    z: 20
    sheetType: MD.Enum.BottomSheetStandard
    dim: false
    dismissOnDragDown: false
    collapsedHeight: 48

    ColumnLayout {
        width: control.sheetWidth
        spacing: 0

        MD.SheetActionBar {
            Layout.fillWidth: true
            delegateWidth: 88
            actions: control.currentWallpaperSelect ? (control.currentWallpaperSelect.actions || []) : []
        }

        MD.Divider {
            Layout.fillWidth: true
            Layout.leftMargin: 16
            Layout.rightMargin: 16
            visible: control.relay.currentComponent !== null
        }

        Loader {
            Layout.fillWidth: true
            visible: control.relay.currentComponent !== null
            sourceComponent: visible ? control.relay.currentComponent : null
        }
    }
}

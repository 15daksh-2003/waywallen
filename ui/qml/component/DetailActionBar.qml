pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD

Item {
    id: root

    property list<MD.Action> actions
    readonly property real targetWidth: Math.ceil(actionToolBar.maximumContentWidth) + 2

    implicitWidth: targetWidth
    implicitHeight: actionToolBar.implicitHeight
    Layout.minimumWidth: targetWidth
    Layout.preferredWidth: targetWidth
    Layout.maximumWidth: targetWidth
    Layout.preferredHeight: actionToolBar.implicitHeight
    Layout.alignment: Qt.AlignVCenter

    MD.ActionToolBar {
        id: actionToolBar
        anchors.fill: parent
        actions: root.actions
        iconDelegate: MD.SmallIconButton {
            id: button

            readonly property string toolTipText: button.action?.text || ""

            action: MD.ToolBarLayout.action
            hoverEnabled: true
            MD.ToolTip.text: button.toolTipText
            MD.ToolTip.visible: button.hovered && button.toolTipText.length > 0 && !button.pressed
        }
    }
}

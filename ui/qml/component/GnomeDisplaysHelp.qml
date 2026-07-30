pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD
import waywallen.ui as W

ColumnLayout {
    id: root

    readonly property string githubUrl: "https://github.com/waywallen/waywallen-display"

    spacing: 12
    visible: W.Util.desktop === W.Util.Desktop.Gnome

    MD.Text {
        Layout.fillWidth: true
        text: qsTr("GNOME Shell needs the <b>waywallen-display</b> extension to bridge wallpapers to the desktop. Install it from GitHub:")
        textFormat: Text.StyledText
        wrapMode: Text.WordWrap
        horizontalAlignment: Text.AlignHCenter
        typescale: MD.Token.typescale.body_medium
        color: MD.Token.color.on_surface
    }

    MD.Button {
        Layout.alignment: Qt.AlignHCenter
        text: qsTr("GitHub")
        mdState.type: MD.Enum.BtFilledTonal
        onClicked: MD.Util.openUrlExternally(root.githubUrl)
        MD.ToolTip.visible: hovered
        MD.ToolTip.text: root.githubUrl
    }
}

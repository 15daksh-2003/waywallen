pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD

ColumnLayout {
    id: control

    required property var sheetState

    width: parent ? parent.width : implicitWidth
    Layout.fillWidth: true
    spacing: 0

    ColumnLayout {
        Layout.fillWidth: true
        Layout.topMargin: 16
        Layout.leftMargin: 16
        Layout.rightMargin: 16
        Layout.bottomMargin: 16
        spacing: 8

        MD.Text {
            Layout.fillWidth: true
            text: qsTr("New playlist")
            typescale: MD.Token.typescale.title_medium
            color: MD.Token.color.on_surface
            maximumLineCount: 1
            elide: Text.ElideRight
        }

        MD.TextField {
            id: newPlaylistNameField
            Layout.fillWidth: true
            mdState.size: MD.Enum.S
            placeholderText: qsTr("Name")
            onAccepted: if (createPlaylistButton.enabled) control.sheetState.createPlaylist(text)
        }

        MD.BusyButton {
            id: createPlaylistButton
            Layout.fillWidth: true
            text: qsTr("Create")
            icon.name: MD.Token.icon.playlist_add
            busy: control.sheetState.mutationQuerying
            enabled: control.sheetState.selectedWallpaperCount > 0
                  && !control.sheetState.mutationQuerying
            mdState.type: MD.Enum.BtFilled
            onClicked: control.sheetState.createPlaylist(newPlaylistNameField.text)
        }
    }
}

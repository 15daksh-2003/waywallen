pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD
import waywallen.ui as W

MD.BottomSheet {
    id: control

    required property Item popupParent
    required property var sheetState

    signal released(var sheet)

    parent: popupParent
    anchors.fill: parent
    z: 30
    sheetType: MD.Enum.BottomSheetModal
    dismissOnDragDown: true
    maxSheetWidth: 560

    onClosed: released(control)

    ColumnLayout {
        width: control.sheetWidth
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 16
            Layout.rightMargin: 16
            Layout.bottomMargin: 8

            MD.Text {
                Layout.fillWidth: true
                text: qsTr("Playlists")
                typescale: MD.Token.typescale.title_medium
                color: MD.Token.color.on_surface
                elide: Text.ElideRight
                maximumLineCount: 1
            }

            MD.EmbedChip {
                id: playlistDisplayChip

                Layout.maximumWidth: 280
                text: control.sheetState.selectedDisplay
                    ? control.sheetState.displayLabel(control.sheetState.selectedDisplay)
                    : qsTr("No displays")
                enabled: control.sheetState.playDisplays.length > 0
                icon.name: MD.Token.icon.monitor
                trailingIconName: MD.Token.icon.expand_more
                mdState.borderWidth: 1
                onClicked: playlistDisplayMenu.open()

                MD.Menu {
                    id: playlistDisplayMenu
                    parent: playlistDisplayChip
                    width: 280
                    x: parent.width - width
                    y: -height
                    model: control.sheetState.playDisplays
                    contentDelegate: MD.MenuItem {
                        required property var modelData
                        text: control.sheetState.displayLabel(modelData)
                        icon.name: String(modelData.id) === String(control.sheetState.selectedDisplayId)
                            ? MD.Token.icon.check
                            : " "
                        onClicked: {
                            control.sheetState.selectDisplay(modelData);
                            playlistDisplayMenu.close();
                        }
                    }
                }
            }
        }

        MD.LinearIndicator {
            Layout.fillWidth: true
            Layout.leftMargin: 16
            Layout.rightMargin: 16
            Layout.bottomMargin: 8
            visible: control.sheetState.listLoading
            running: visible
        }

        MD.Text {
            Layout.fillWidth: true
            Layout.preferredHeight: 96
            Layout.leftMargin: 16
            Layout.rightMargin: 16
            visible: !control.sheetState.listLoading
                  && control.sheetState.playlists.length === 0
            text: qsTr("No playlists found")
            typescale: MD.Token.typescale.body_large
            color: MD.Token.color.on_surface_variant
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }

        MD.VerticalListView {
            id: playlistSheetList
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(360, Math.max(120, contentHeight + topMargin + bottomMargin))
            visible: control.sheetState.playlists.length > 0
            interactive: contentHeight + topMargin + bottomMargin > height
            model: control.sheetState.playlists
            spacing: 6
            leftMargin: 16
            rightMargin: 16
            topMargin: 0
            bottomMargin: 16

            delegate: MD.ListItem {
                id: playlistSheetItem
                required property var modelData

                width: ListView.view.contentWidth
                radius: 12
                text: modelData.name || qsTr("Untitled")
                supportText: qsTr("%1 wallpapers").arg((modelData.entryIds || []).length)
                heightMode: playingDisplayLabels.length > 0
                    ? MD.Enum.ListItemThreeLine
                    : MD.Enum.ListItemTwoLine
                readonly property bool playingOnSelectedDisplay:
                    control.sheetState.playlistIsPlayingOnSelectedDisplay(modelData)
                readonly property var playingDisplayLabels:
                    control.sheetState.playlistDisplayLabels(modelData)
                mdState.backgroundColor: control.sheetState.isEditingPlaylist(modelData)
                    ? MD.Token.color.primary_container
                    : MD.Token.color.surface_container

                below: Item {
                    implicitHeight: tagFlow.visible ? tagFlow.implicitHeight + 6 : 0

                    Flow {
                        id: tagFlow
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.topMargin: 6
                        spacing: 4
                        visible: playlistSheetItem.playingDisplayLabels.length > 0

                        Repeater {
                            model: playlistSheetItem.playingDisplayLabels

                            W.Tag {
                                required property var modelData
                                text: modelData
                                bgColor: MD.Token.color.secondary_container
                                fgColor: MD.Token.color.on_secondary_container
                            }
                        }
                    }
                }

                trailing: RowLayout {
                    spacing: 4

                    MD.BusyIconButton {
                        enabled: control.sheetState.selectedDisplay !== null
                              && !control.sheetState.playlistPlaybackMutation.querying
                        busy: control.sheetState.playlistPlaybackMutation.querying
                        icon.name: playlistSheetItem.playingOnSelectedDisplay
                            ? MD.Token.icon.pause
                            : MD.Token.icon.play_arrow
                        onClicked: control.sheetState.togglePlayback(playlistSheetItem.modelData)

                        MD.ToolTip {
                            visible: parent.hovered && !parent.enabled
                            text: qsTr("No displays")
                        }
                    }

                    MD.IconButton {
                        enabled: !control.sheetState.playlistMutation.querying
                        icon.name: MD.Token.icon.edit
                        onClicked: control.sheetState.editSelection(playlistSheetItem.modelData)

                        MD.ToolTip {
                            visible: parent.hovered
                            text: qsTr("Edit selection")
                        }
                    }

                    MD.IconButton {
                        enabled: !control.sheetState.playlistMutation.querying
                        icon.name: MD.Token.icon.delete
                        onClicked: control.sheetState.deletePlaylist(playlistSheetItem.modelData)

                        MD.ToolTip {
                            visible: parent.hovered
                            text: qsTr("Delete playlist")
                        }
                    }
                }
            }
        }
    }
}

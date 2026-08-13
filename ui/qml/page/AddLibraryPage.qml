pragma ValueTypeBehavior: Assertable
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD
import waywallen.ui as W

MD.Page {
    id: root
    title: qsTr("Add Library")

    W.SourceListQuery {
        id: sourceQuery
    }

    // `daemonReady` is edge-triggered, so also level-check in
    // Component.onCompleted for the case where this page is pushed
    // after the daemon is already Ready. Without the ready gate an
    // early reload returns nothing and the source plugin list stays
    // empty with no retry.
    Connections {
        target: W.Notify
        function onDaemonReady() {
            sourceQuery.reload();
        }
        function onPluginChanged() {
            sourceQuery.reload();
        }
    }

    Component.onCompleted: {
        if (W.Notify.daemonPhase === W.Notify.DaemonPhase.Ready)
            sourceQuery.reload();
    }

    W.LibraryAddQuery {
        id: addQuery
        // Toast now fires globally from Window.qml on the daemon's
        // `LibrariesAdded` event (mirrored via Notify).
        onFinished: MD.Util.closePopup(root.MD.MProp.page)
    }

    MD.ButtonGroup {
        id: pluginGroup
        exclusive: true
        property string selectedPlugin: ""
    }

    readonly property var selectedSource: {
        const list = sourceQuery.sources || [];
        for (let i = 0; i < list.length; ++i) {
            if (list[i].name === pluginGroup.selectedPlugin) return list[i];
        }
        return null;
    }

    MD.FolderDialog {
        id: folderDialog
        title: qsTr("Choose Library Folder")
        onAccepted: {
            pathInput.text = selectedFolder.toString().replace(/^file:\/\//, "");
        }
    }

    contentItem: MD.Flickable {
        implicitHeight: contentHeight
        contentHeight: contentCol.implicitHeight
        leftMargin: 12
        rightMargin: 12

        ColumnLayout {
            id: contentCol
            width: parent.width
            spacing: 16
            Layout.margins: 16

            MD.Text {
                text: qsTr("Select Source Plugin")
                typescale: MD.Token.typescale.title_small
            }

            Flow {
                Layout.fillWidth: true
                spacing: 8

                Repeater {
                    model: sourceQuery.sources
                    delegate: MD.FilterChip {
                        required property var modelData
                        MD.ButtonGroup.group: pluginGroup
                        text: modelData.name
                        onClicked: pluginGroup.selectedPlugin = checked ? modelData.name : ""
                    }
                }
            }

            MD.Divider {
                Layout.fillWidth: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                MD.TextField {
                    id: pathInput
                    Layout.fillWidth: true
                    mdState.size: MD.Enum.S
                    placeholderText: (root.selectedSource && root.selectedSource.libraryLabel)
                                     ? root.selectedSource.libraryLabel
                                     : qsTr("Library Path")
                }

                MD.IconButton {
                    Layout.alignment: Qt.AlignVCenter
                    icon.name: MD.Token.icon.folder
                    onClicked: folderDialog.open()
                }
            }

            MD.Text {
                Layout.fillWidth: true
                visible: root.selectedSource && root.selectedSource.libraryHint
                         && root.selectedSource.libraryHint.length > 0
                text: root.selectedSource ? (root.selectedSource.libraryHint || "") : ""
                wrapMode: Text.WordWrap
                typescale: MD.Token.typescale.body_small
                color: MD.Token.color.on_surface_variant
            }

            Item {
                Layout.fillHeight: true
            }

            MD.BusyButton {
                Layout.fillWidth: true
                text: qsTr("Add Library")
                busy: addQuery.querying
                enabled: pluginGroup.selectedPlugin !== "" && pathInput.text !== ""
                mdState.type: MD.Enum.BtFilled
                onClicked: {
                    addQuery.pluginName = pluginGroup.selectedPlugin;
                    addQuery.path = pathInput.text;
                    addQuery.reload();
                }
            }

            Item {}
        }
    }
}

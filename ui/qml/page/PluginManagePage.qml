pragma ComponentBehavior: Bound
pragma ValueTypeBehavior: Assertable
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import Qcm.Material as MD
import waywallen.ui as W

MD.Page {
    id: root
    title: 'Plugins'
    scrolling: !m_flick.atYBeginning
    readonly property int inactivePluginCount: (pluginListQuery.inactiveSystem ? pluginListQuery.inactiveSystem.length : 0) + (pluginListQuery.inactiveUser ? pluginListQuery.inactiveUser.length : 0)
    property var inactiveDialog: null

    function openInactiveDialog() {
        if (inactiveDialog && (inactiveDialog.opened || inactiveDialog.entering || inactiveDialog.closing))
            return;
        inactiveDialog = MD.Util.showPopup(inactiveDialogComponent, {}, root.Window.window);
    }

    actions: [
        MD.Action {
            icon.name: MD.Token.icon.warning
            text: qsTr("Inactive plugins")
            property bool visible: root.inactivePluginCount > 0
            onTriggered: root.openInactiveDialog()
        },
        MD.Action {
            icon.name: MD.Token.icon.add
            text: qsTr("Install from .zip")
            enabled: !installQuery.querying
            onTriggered: zipDialog.open()
        }
    ]

    W.PluginListQuery {
        id: pluginListQuery
    }

    W.PluginInstallQuery {
        id: installQuery
    }

    W.PluginDeleteQuery {
        id: deleteQuery
    }

    Connections {
        target: W.Notify
        function onDaemonReady() {
            pluginListQuery.reload();
        }
    }

    Connections {
        target: deleteQuery
        function onDeleted(pluginId, needsRestart) {
            W.Action.toast(needsRestart
                ? qsTr("Deleted \"%1\" — restart waywallen to unload it").arg(pluginId)
                : qsTr("Deleted \"%1\"").arg(pluginId));
            pluginListQuery.reload();
        }
    }

    Connections {
        target: installQuery
        function onInstalled(pluginId, needsRestart) {
            W.Action.toast(needsRestart
                ? qsTr("Installed \"%1\" — restart waywallen to load it").arg(pluginId)
                : qsTr("Installed \"%1\"").arg(pluginId));
            pluginListQuery.reload();
        }
    }

    Component.onCompleted: {
        if (W.Notify.daemonPhase === W.Notify.DaemonPhase.Ready)
            pluginListQuery.reload();
    }

    Component {
        id: inactiveDialogComponent

        MD.Dialog {
            id: dynamicInactiveDialog
            title: qsTr("Inactive plugins")
            parent: T.Overlay.overlay
            horizontalPadding: 16
            implicitWidth: Math.min(440, parent ? parent.width - 48 : 440)
            standardButtons: T.Dialog.Close
            onClosed: {
                if (root.inactiveDialog === dynamicInactiveDialog)
                    root.inactiveDialog = null;
            }

            contentItem: ColumnLayout {
                spacing: 12

                MD.Text {
                    Layout.fillWidth: true
                    text: qsTr("These plugins were skipped because another installed plugin with the same id was selected. Higher versions win; when versions match, user plugins win over system plugins.")
                    typescale: MD.Token.typescale.body_medium
                    color: MD.Token.color.on_surface_variant
                    wrapMode: Text.WordWrap
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    visible: pluginListQuery.inactiveUser && pluginListQuery.inactiveUser.length > 0

                    MD.Text {
                        Layout.fillWidth: true
                        text: qsTr("User")
                        typescale: MD.Token.typescale.title_small
                        color: MD.Token.color.on_surface
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: 6
                        Repeater {
                            model: pluginListQuery.inactiveUser
                            delegate: W.Tag {
                                required property var modelData
                                text: modelData
                            }
                        }
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    visible: pluginListQuery.inactiveSystem && pluginListQuery.inactiveSystem.length > 0

                    MD.Text {
                        Layout.fillWidth: true
                        text: qsTr("System")
                        typescale: MD.Token.typescale.title_small
                        color: MD.Token.color.on_surface
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: 6
                        Repeater {
                            model: pluginListQuery.inactiveSystem
                            delegate: W.Tag {
                                required property var modelData
                                text: modelData
                            }
                        }
                    }
                }
            }
        }
    }

    MD.FileDialog {
        id: zipDialog
        title: qsTr("Choose plugin package")
        fileMode: MD.FileDialog.OpenFile
        nameFilters: ["Plugin package (*.zip)", "All files (*)"]
        onAccepted: {
            installQuery.zipPath = selectedFile.toString().replace(/^file:\/\//, "");
            installQuery.reload();
        }
    }

    contentItem: MD.VerticalFlickable {
        id: m_flick
        leftMargin: 12
        rightMargin: 12
        bottomMargin: 12

        ColumnLayout {
            width: m_flick.contentWidth
            spacing: 8

            MD.Text {
                Layout.fillWidth: true
                visible: !pluginListQuery.plugins || pluginListQuery.plugins.length === 0
                text: "No plugins installed"
                typescale: MD.Token.typescale.body_medium
                color: MD.Token.color.on_surface_variant
                wrapMode: Text.WordWrap
            }

            ListView {
                id: pluginListView
                Layout.fillWidth: true
                Layout.preferredHeight: contentHeight
                implicitHeight: contentHeight
                interactive: false
                spacing: 4

                model: pluginListQuery.plugins

                section.property: "section"
                section.criteria: ViewSection.FullString
                section.delegate: MD.Text {
                    required property string section
                    width: pluginListView.width
                    text: section === "user" ? qsTr("User") : qsTr("System")
                    typescale: MD.Token.typescale.title_small
                    color: MD.Token.color.on_surface_variant
                    topPadding: 4
                    bottomPadding: 4
                    leftPadding: 4
                }

                delegate: MD.ListItem {
                    id: pluginItem
                    required property var modelData

                    width: ListView.view.width
                    radius: 12
                    mdState.backgroundColor: MD.Token.color.surface_container
                    text: modelData.name || modelData.id || ""
                    supportText: modelData.id
                    leader: MD.Icon {
                        name: MD.Token.icon.extension
                        size: 24
                        color: MD.Token.color.on_surface_variant
                    }
                    trailing: RowLayout {
                        spacing: 6
                        W.Tag {
                            Layout.alignment: Qt.AlignVCenter
                            text: "v" + (pluginItem.modelData.version || "0.0.0")
                        }
                        MD.IconButton {
                            Layout.alignment: Qt.AlignVCenter
                            visible: pluginItem.modelData.system !== true
                            enabled: !deleteQuery.querying
                            icon.name: MD.Token.icon.delete
                            onClicked: deleteQuery.remove(pluginItem.modelData.id)
                        }
                    }
                    below: Flow {
                        spacing: 6
                        bottomPadding: 8
                        Repeater {
                            model: pluginItem.modelData.renderers
                            delegate: W.Tag {
                                required property var modelData
                                text: modelData.name || ""
                            }
                        }
                    }
                }
            }
        }
    }
}

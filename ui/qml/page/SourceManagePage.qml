pragma ComponentBehavior: Bound
pragma ValueTypeBehavior: Assertable
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD
import waywallen.ui as W

MD.Page {
    id: root
    title: qsTr("Library Manager")

    actions: [
        MD.Action {
            icon.name: MD.Token.icon.add
            text: qsTr("Add library")
            onTriggered: root.MD.MProp.page.pushItem('waywallen.ui/AddLibraryPage')
        }
    ]

    W.LibraryRemoveQuery {
        id: removeQuery
    }

    contentItem: Item {
        implicitHeight: Math.max(m_view.contentHeight, emptyLabel.implicitHeight) + 16
        implicitWidth: m_view.implicitWidth

        MD.VerticalListView {
            id: m_view
            width: parent.width
            height: parent.height
            model: W.App.libraryManager.libraries
            spacing: 8

            topMargin: 8
            bottomMargin: 8
            leftMargin: 12
            rightMargin: 12

            delegate: MD.ListItem {
                id: sourceItem
                required property var modelData
                readonly property string fullPath: String(modelData.path || "")

                width: m_view.contentWidth
                radius: 8

                mdState.backgroundColor: MD.Token.color.surface_container

                text: fullPath
                wrapMode: Text.Wrap
                maximumLineCount: 3
                supportText: qsTr("Plugin: %1").arg(modelData.pluginName)

                leader: MD.Icon {
                    name: MD.Token.icon.folder
                    color: MD.Token.color.on_surface_variant
                }

                trailing: MD.IconButton {
                    icon.name: MD.Token.icon.delete
                    onClicked: {
                        removeQuery.libraryId = modelData.id;
                        removeQuery.reload();
                    }
                }
            }
        }

        MD.Text {
            id: emptyLabel
            visible: W.App.libraryManager.count === 0
            anchors.centerIn: parent
            width: Math.max(0, parent.width - 24)
            text: qsTr("No libraries")
            typescale: MD.Token.typescale.body_medium
            color: MD.Token.color.on_surface_variant
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
        }
    }
}

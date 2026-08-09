pragma ValueTypeBehavior: Assertable
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import Qcm.Material as MD

MD.Page {
    id: root
    padding: 0

    contentItem: MD.VerticalFlickable {
        id: aboutFlick
        topMargin: 24
        bottomMargin: 24
        leftMargin: 16
        rightMargin: 16

        T.ScrollBar.vertical: MD.ScrollBar {}

        ColumnLayout {
            x: (aboutFlick.contentWidth - width) / 2
            width: Math.min(aboutFlick.contentWidth, 600)
            spacing: 16

            Image {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 96
                Layout.preferredHeight: 96
                source: "qrc:/waywallen/ui/assets/waywallen-ui.svg"
                fillMode: Image.PreserveAspectFit
                visible: status === Image.Ready
            }

            MD.Text {
                Layout.alignment: Qt.AlignHCenter
                text: "waywallen"
                typescale: MD.Token.typescale.headline_large
                color: MD.Token.color.on_surface
            }

            MD.Text {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Version %1").arg(Qt.application.version)
                typescale: MD.Token.typescale.body_medium
                color: MD.Token.color.on_surface_variant
            }

            Item {
                Layout.alignment: Qt.AlignHCenter
                implicitWidth: m_author_button.implicitWidth
                implicitHeight: m_author_button.contentItem.implicitHeight

                MD.Button {
                    id: m_author_button
                    anchors.centerIn: parent
                    text: "hypengw"
                    mdState.type: MD.Enum.BtText
                    onClicked: MD.Util.openUrlExternally("https://github.com/hypengw")
                }
            }

            MD.Text {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Wallpaper Manager for Linux")
                typescale: MD.Token.typescale.body_large
                color: MD.Token.color.on_surface
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            MD.Text {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Waywallen is a dynamic wallpaper solution for Linux desktops.")
                typescale: MD.Token.typescale.body_medium
                color: MD.Token.color.on_surface_variant
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            MD.Divider {
                Layout.fillWidth: true
                Layout.topMargin: 8
                Layout.bottomMargin: 8
            }

            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: 24

                MD.Button {
                    text: qsTr("GitHub")
                    mdState.type: MD.Enum.BtText
                    onClicked: MD.Util.openUrlExternally("https://github.com/waywallen")
                }

                MD.Button {
                    text: qsTr("Issues")
                    mdState.type: MD.Enum.BtText
                    onClicked: MD.Util.openUrlExternally("https://github.com/waywallen/waywallen/issues")
                }

                MD.Button {
                    text: qsTr("Donate")
                    mdState.type: MD.Enum.BtText
                    onClicked: MD.Util.openUrlExternally("https://ko-fi.com/hypengw")
                }
            }

            MD.Divider {
                Layout.fillWidth: true
                Layout.topMargin: 8
                Layout.bottomMargin: 8
            }

            MD.Text {
                Layout.fillWidth: true
                text: qsTr("Changelog")
                typescale: MD.Token.typescale.headline_small
                color: MD.Token.color.on_surface
            }

            MD.Changelog {
                Layout.fillWidth: true
                expand: true
                source: "qrc:/waywallen/ui/assets/waywallen-ui.releases.xml"
            }
        }
    }
}

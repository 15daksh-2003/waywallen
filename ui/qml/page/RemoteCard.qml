pragma ComponentBehavior: Bound
import QtQuick
import Qcm.Material as MD

Item {
    id: root

    required property int index
    required property string itemId
    required property string title
    required property string previewUrl
    required property string author
    required property int acquisitionState
    required property int remoteCapability
    property real itemWidth: width
    property real itemHeight: height

    signal clicked()

    width: GridView.view ? GridView.view.cellWidth : 0
    height: GridView.view ? GridView.view.cellHeight : 0

    readonly property int _radius: MD.Token.shape.corner.extra_small
    readonly property real cardWidth: Math.min(root.itemWidth, root.width)
    readonly property real cardHeight: Math.min(root.itemHeight, root.height)

    Item {
        id: m_card
        width: root.cardWidth
        height: root.cardHeight
        anchors.centerIn: parent

        Item {
            id: m_cell
            anchors.fill: parent
            anchors.margins: 6
            clip: true

            AnimatedImage {
                id: m_thumb
                anchors.fill: parent
                source: root.previewUrl
                fillMode: Image.PreserveAspectCrop
                horizontalAlignment: Image.AlignHCenter
                verticalAlignment: Image.AlignVCenter
                smooth: true
                cache: true
                playing: true
                asynchronous: true
                onStatusChanged: if (status === AnimatedImage.Ready) playing = true
                layer.enabled: true
                layer.effect: MD.RoundClip {
                    corners: MD.Util.corners(root._radius)
                    size: Qt.vector2d(m_thumb.width, m_thumb.height)
                }
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Math.max(0, parent.height - m_title.y)
                visible: height > 0
                radius: root._radius
                gradient: Gradient {
                    GradientStop { position: 0.0; color: "transparent" }
                    GradientStop { position: 1.0; color: Qt.rgba(0, 0, 0, 0.65) }
                }
            }

            MD.Text {
                id: m_title
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 6
                text: root.title.length > 0 ? root.title : qsTr("Untitled")
                typescale: MD.Token.typescale.title_small
                color: "white"
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                elide: Text.ElideRight
                maximumLineCount: 2
                leftPadding: 8
                rightPadding: 8
            }

            Rectangle {
                visible: (root.remoteCapability === 1 && root.acquisitionState === 3)
                    || (root.remoteCapability === 2 && root.acquisitionState === 2)
                anchors { top: parent.top; right: parent.right; margins: 6 }
                width: m_badge.implicitWidth + 12
                height: m_badge.implicitHeight + 6
                radius: height / 2
                color: MD.Token.color.primary

                MD.Label {
                    id: m_badge
                    anchors.centerIn: parent
                    text: root.remoteCapability === 2 ? qsTr("Subscribed") : qsTr("Downloaded")
                    typescale: MD.Token.typescale.label_small
                    color: MD.Token.color.on_primary
                }
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.clicked()
            }
        }
    }
}

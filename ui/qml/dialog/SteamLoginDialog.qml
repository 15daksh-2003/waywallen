pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T

import Qcm.Material as MD
import waywallen.ui as W

// In-UI Steam login. A sign-in action starts the daemon's QR flow;
// SteamLoginState arrives via Notify: 1 STARTING, 2 AWAITING_SCAN, 3 SUCCESS,
// 4 FAILED, 5 CANCELLED. qrImage is an SVG data URL the daemon renders from the
// Steam challenge URL that the Steam mobile app scans to approve the sign-in.
MD.Popup {
    id: root

    property int loginState: 0
    property string qrImage: ""
    property string account: ""
    property string errorText: ""

    closePolicy: T.Popup.CloseOnEscape
    dim: true
    modal: true
    parent: T.Overlay.overlay
    x: Math.round((parent.width - width) / 2)
    y: Math.round((parent.height - height) / 2)
    bottomPadding: 24

    W.SteamLoginCancelQuery {
        id: cancelQ
    }

    onClosed: {
        // Abort an in-flight login if the user dismissed before it finished.
        if (root.loginState !== 3 && root.loginState !== 5)
            cancelQ.reload();
    }

    // Passive: a sign-in action elsewhere starts the daemon flow; this dialog
    // opens itself when progress begins and closes on a terminal state.
    Connections {
        target: W.Notify
        function onSteamLoginProgress(state, qrImage, accountName, error) {
            root.loginState = state;
            root.qrImage = qrImage;
            root.account = accountName;
            root.errorText = error;
            if ((state === 1 || state === 2) && !root.visible)
                root.open();
            if (state === 3) {
                W.Action.toast(accountName.length > 0
                    ? qsTr("Signed in to Steam as %1").arg(accountName)
                    : qsTr("Signed in to Steam"));
                root.close();
            } else if (state === 5) {
                root.close();
            }
        }
    }

    contentItem: ColumnLayout {
        spacing: 16

        MD.DialogHeader {
            Layout.fillWidth: true
            title: qsTr("Sign in to Steam")
        }

        MD.Label {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            wrapMode: Text.WordWrap
            visible: root.loginState <= 1
            text: qsTr("Starting Steam sign-in…")
        }

        MD.LinearIndicator {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            visible: root.loginState <= 1
        }

        Rectangle {
            Layout.alignment: Qt.AlignHCenter
            visible: root.loginState === 2
            color: "white"
            implicitWidth: 256 + 24
            implicitHeight: 256 + 24

            Image {
                anchors.centerIn: parent
                sourceSize.width: 256
                sourceSize.height: 256
                width: 256
                height: 256
                fillMode: Image.PreserveAspectFit
                smooth: false
                source: root.qrImage
            }
        }

        MD.Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            visible: root.loginState === 2
            text: qsTr("Scan with the Steam mobile app")
        }

        MD.Label {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            wrapMode: Text.WordWrap
            visible: root.loginState === 4
            text: qsTr("Sign-in failed: %1").arg(root.errorText)
        }

        MD.DialogButtonBox {
            Layout.fillWidth: true

            MD.Button {
                text: root.loginState === 4 ? qsTr("Close") : qsTr("Cancel")
                mdState.type: MD.Enum.BtText
                T.DialogButtonBox.buttonRole: T.DialogButtonBox.RejectRole
                onClicked: root.close()
            }
        }
    }
}

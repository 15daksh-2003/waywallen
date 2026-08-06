pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T

import Qcm.Material as MD
import waywallen.ui as W

MD.Popup {
    id: root

    property string sessionId: ""
    property string pluginId: ""
    property string actionId: ""
    property int loginState: 0
    property string qrImage: ""
    property string displayValue: ""
    property string errorText: ""
    property string dialogTitle: ""
    property string instruction: ""

    closePolicy: T.Popup.CloseOnEscape
    dim: true
    modal: true
    parent: T.Overlay.overlay
    x: Math.round((parent.width - width) / 2)
    y: Math.round((parent.height - height) / 2)
    bottomPadding: 24

    W.QrLoginCancelQuery {
        id: cancelQuery
        sessionId: root.sessionId
    }

    onClosed: {
        if (root.loginState >= 1 && root.loginState <= 4)
            cancelQuery.reload();
    }

    Connections {
        target: W.Notify
        function onQrLoginProgress(sessionId, pluginId, actionId, state, qrImage,
                                   displayValue, error, title, instruction) {
            if (root.visible && root.sessionId.length > 0 && root.sessionId !== sessionId)
                return;
            if (state === 1) {
                root.sessionId = sessionId;
                root.pluginId = pluginId;
                root.actionId = actionId;
                root.qrImage = "";
                root.displayValue = "";
                root.errorText = "";
                root.dialogTitle = "";
                root.instruction = "";
            }
            root.loginState = state;
            if (qrImage.length > 0)
                root.qrImage = qrImage;
            if (displayValue.length > 0)
                root.displayValue = displayValue;
            if (error.length > 0)
                root.errorText = error;
            if (title.length > 0)
                root.dialogTitle = title;
            if (instruction.length > 0)
                root.instruction = instruction;
            if (state >= 1 && state <= 4 && !root.visible)
                root.open();
            if (state === 5) {
                W.Action.toast(displayValue.length > 0
                    ? qsTr("Signed in as %1").arg(displayValue)
                    : qsTr("Signed in"));
                root.close();
            } else if (state === 6 || state === 7) {
                W.Global.toastError(error.length > 0
                    ? error
                    : (state === 6 ? qsTr("Sign-in expired") : qsTr("Sign-in failed")));
                root.close();
            } else if (state === 8) {
                root.close();
            }
        }
    }

    contentItem: ColumnLayout {
        spacing: 16

        MD.DialogHeader {
            Layout.fillWidth: true
            title: root.dialogTitle.length > 0 ? root.dialogTitle : qsTr("Sign in")
        }

        MD.Label {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            wrapMode: Text.WordWrap
            visible: root.loginState === 1
            text: qsTr("Starting sign-in…")
        }

        MD.LinearIndicator {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            visible: root.loginState === 1 || root.loginState === 3
        }

        Rectangle {
            Layout.alignment: Qt.AlignHCenter
            visible: root.loginState === 2 || root.loginState === 4
            color: "white"
            implicitWidth: 280
            implicitHeight: 280

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
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            visible: root.loginState === 2 || root.loginState === 4
            text: root.instruction.length > 0 ? root.instruction : qsTr("Scan the QR code to continue")
        }

        MD.Label {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            visible: root.loginState === 3
            text: root.displayValue.length > 0
                ? root.displayValue
                : qsTr("Waiting for confirmation…")
        }

        MD.Label {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            wrapMode: Text.WordWrap
            visible: root.loginState === 6 || root.loginState === 7
            text: root.errorText.length > 0
                ? root.errorText
                : (root.loginState === 6 ? qsTr("Sign-in expired") : qsTr("Sign-in failed"))
            color: MD.Token.color.error
        }

        MD.DialogButtonBox {
            Layout.fillWidth: true

            MD.Button {
                text: root.loginState === 6 || root.loginState === 7
                    ? qsTr("Close") : qsTr("Cancel")
                mdState.type: MD.Enum.BtText
                T.DialogButtonBox.buttonRole: T.DialogButtonBox.RejectRole
                onClicked: root.close()
            }
        }
    }
}

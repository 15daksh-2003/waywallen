pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Templates as T
import Qcm.Material as MD

MD.Dialog {
    id: control

    property string displayName: ""
    property string systemName: ""
    property var displayId: 0
    property string serverAlias: ""
    property string originalValue: ""
    property string pendingAlias: ""
    property bool resetRequested: false
    readonly property bool hasChanges: resetRequested ? serverAlias.length > 0 : pendingAlias.trim() !== originalValue
    readonly property bool canReset: !resetRequested && (serverAlias.length > 0 || pendingAlias.trim() !== systemName)

    signal submitted(string name, var targetId, string alias, bool clear)

    title: qsTr("Edit display")
    parent: T.Overlay.overlay
    modal: true
    horizontalPadding: 24
    implicitWidth: Math.min(400, parent ? parent.width - 48 : 400)
    standardButtons: T.Dialog.Cancel | T.Dialog.Reset | T.Dialog.Ok

    function openFor(display) {
        if (!display)
            return;
        displayName = display.name || "";
        displayId = display.id;
        systemName = displayName || qsTr("Display %1").arg(displayId);
        serverAlias = display.alias || "";
        originalValue = serverAlias || systemName;
        pendingAlias = originalValue;
        resetRequested = false;
        displayNameField.text = pendingAlias;
        open();
    }

    Component.onCompleted: {
        const save = standardButton(T.Dialog.Ok);
        if (save) {
            save.text = qsTr("Save");
            save.enabled = Qt.binding(function () {
                return control.hasChanges;
            });
        }
        const reset = standardButton(T.Dialog.Reset);
        if (reset)
            reset.enabled = Qt.binding(function () {
                return control.canReset;
            });
    }

    onOpened: displayNameField.forceActiveFocus()
    onReset: {
        pendingAlias = systemName;
        resetRequested = true;
        displayNameField.text = pendingAlias;
    }
    onAccepted: {
        const alias = pendingAlias.trim();
        const clear = resetRequested || alias.length === 0;
        submitted(displayName, displayId, clear ? "" : alias, clear);
    }

    contentItem: MD.TextField {
        id: displayNameField
        mdState.size: MD.Enum.S
        placeholderText: qsTr("Display name")
        onTextEdited: {
            control.pendingAlias = text;
            control.resetRequested = false;
        }
        onAccepted: if (control.hasChanges)
            control.accept()
    }
}

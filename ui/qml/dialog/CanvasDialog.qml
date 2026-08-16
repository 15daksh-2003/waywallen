pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Templates as T
import Qcm.Material as MD

MD.Dialog {
    id: control

    property string originalName: ""
    property string pendingName: ""
    readonly property bool hasChanges: pendingName.trim() !== originalName

    signal submitted(string name)

    title: qsTr("Edit canvas")
    parent: T.Overlay.overlay
    modal: true
    horizontalPadding: 24
    implicitWidth: Math.min(400, parent ? parent.width - 48 : 400)
    standardButtons: T.Dialog.Cancel | T.Dialog.Ok

    function openFor(canvas) {
        if (!canvas)
            return;
        originalName = canvas.name || "";
        pendingName = originalName;
        canvasName.text = pendingName;
        open();
    }

    Component.onCompleted: {
        const save = standardButton(T.Dialog.Ok);
        if (save) {
            save.text = qsTr("Save");
            save.enabled = Qt.binding(function () {
                return control.hasChanges && control.pendingName.trim().length > 0;
            });
        }
    }

    onOpened: canvasName.forceActiveFocus()
    onAccepted: submitted(pendingName.trim())

    contentItem: MD.TextField {
        id: canvasName
        mdState.size: MD.Enum.S
        placeholderText: qsTr("Canvas name")
        onTextEdited: control.pendingName = text
        onAccepted: if (control.hasChanges && control.pendingName.trim().length > 0) {
            control.accept();
        }
    }
}

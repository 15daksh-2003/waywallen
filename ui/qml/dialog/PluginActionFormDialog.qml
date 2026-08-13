pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import Qcm.Material as MD

MD.Dialog {
    id: root

    property var action: ({})
    property var values: ({})
    property int resetRevision: 0
    signal submitted(var values)

    readonly property var fields: action.fields || []
    readonly property bool complete: {
        for (let i = 0; i < fields.length; ++i) {
            if (fields[i].required && String(values[fields[i].key] || "").length === 0)
                return false;
        }
        return true;
    }

    title: action.label || qsTr("Continue")
    parent: T.Overlay.overlay
    modal: true
    horizontalPadding: 24
    implicitWidth: Math.min(440, parent ? parent.width - 48 : 440)
    standardButtons: T.Dialog.Cancel | T.Dialog.Ok

    function openFor(nextAction) {
        action = nextAction || ({});
        values = ({});
        resetRevision += 1;
        open();
    }

    function setValue(key, value) {
        const next = Object.assign({}, values);
        next[key] = value;
        values = next;
    }

    onAboutToShow: {
        const accept = standardButton(T.Dialog.Ok);
        if (accept) {
            accept.text = action.label || qsTr("Continue");
            accept.enabled = Qt.binding(function() { return root.complete; });
        }
    }
    onAccepted: submitted(Object.assign({}, values))

    contentItem: ColumnLayout {
        spacing: 12

        MD.Text {
            Layout.fillWidth: true
            visible: String(root.action.description || "").length > 0
            text: root.action.description || ""
            typescale: MD.Token.typescale.body_medium
            color: MD.Token.color.on_surface_variant
            wrapMode: Text.WordWrap
        }

        Repeater {
            model: root.fields
            delegate: ColumnLayout {
                id: fieldItem
                required property var modelData
                Layout.fillWidth: true
                spacing: 4

                MD.Text {
                    Layout.fillWidth: true
                    text: fieldItem.modelData.label || fieldItem.modelData.key
                    typescale: MD.Token.typescale.label_large
                    color: MD.Token.color.on_surface
                }

                MD.TextField {
                    id: input
                    Layout.fillWidth: true
                    mdState.size: MD.Enum.S
                    placeholderText: fieldItem.modelData.placeholder || ""
                    echoMode: fieldItem.modelData.secret ? TextInput.Password : TextInput.Normal
                    inputMethodHints: fieldItem.modelData.secret
                        ? Qt.ImhSensitiveData | Qt.ImhNoPredictiveText
                        : Qt.ImhNone
                    onTextEdited: root.setValue(fieldItem.modelData.key, text)

                    Connections {
                        target: root
                        function onResetRevisionChanged() {
                            input.text = "";
                        }
                    }
                }

                MD.Text {
                    Layout.fillWidth: true
                    visible: String(fieldItem.modelData.description || "").length > 0
                    text: fieldItem.modelData.description || ""
                    typescale: MD.Token.typescale.body_small
                    color: MD.Token.color.on_surface_variant
                    wrapMode: Text.WordWrap
                }
            }
        }
    }
}

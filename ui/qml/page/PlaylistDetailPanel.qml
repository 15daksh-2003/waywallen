pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD

ColumnLayout {
    id: control

    property var playlist: null
    property var mutation: null

    spacing: 0

    component FieldLabel: MD.Text {
        typescale: MD.Token.typescale.label_medium
        color: MD.Token.color.on_surface_variant
    }

    component SectionPane: MD.Pane {
        Layout.fillWidth: true
        padding: 0
        showBackground: false
    }

    function canMutate() {
        return !!control.playlist && !!control.mutation && control.mutation.querying !== true;
    }

    function ivalH() {
        return control.playlist ? Math.floor(control.playlist.intervalSecs / 3600) : 0;
    }

    function ivalM() {
        return control.playlist ? Math.floor((control.playlist.intervalSecs % 3600) / 60) : 0;
    }

    function ivalS() {
        return control.playlist ? (control.playlist.intervalSecs % 60) : 0;
    }

    function applyInterval() {
        if (!control.canMutate())
            return;

        let h = parseInt(fieldH.text) || 0;
        let m = parseInt(fieldM.text) || 0;
        let s = parseInt(fieldS.text) || 0;
        let total = h * 3600 + m * 60 + s;
        if (total < 10)
            total = 10;
        control.mutation.setInterval(control.playlist.id, total);
    }

    ColumnLayout {
        id: m_detail

        Layout.fillWidth: true
        Layout.topMargin: 16
        Layout.leftMargin: 16
        Layout.rightMargin: 16
        Layout.bottomMargin: 16
        spacing: 12

        SectionPane {
            contentItem: ColumnLayout {
                spacing: 12

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4

                    MD.TextField {
                        Layout.fillWidth: true
                        mdState.size: MD.Enum.S
                        enabled: control.canMutate()
                        placeholderText: qsTr("Name")
                        text: control.playlist ? control.playlist.name : ""
                        onEditingFinished: {
                            if (control.canMutate())
                                control.mutation.rename(control.playlist.id, text);
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 12

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.minimumWidth: 144
                        spacing: 4

                        FieldLabel { text: qsTr("Mode") }

                        MD.ComboBox {
                            Layout.fillWidth: true
                            mdState.size: MD.Enum.S
                            enabled: control.canMutate()
                            model: [qsTr("Sequential"), qsTr("Shuffle"), qsTr("Random")]
                            currentIndex: control.playlist ? Math.max(0, control.playlist.mode - 1) : 0
                            onActivated: {
                                if (control.canMutate())
                                    control.mutation.setMode(control.playlist.id, currentIndex + 1);
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.alignment: Qt.AlignTop
                        Layout.minimumWidth: intervalRow.implicitWidth
                        spacing: 4

                        FieldLabel { text: qsTr("Rotation interval") }

                        RowLayout {
                            id: intervalRow

                            spacing: 8

                            MD.TextField {
                                id: fieldH

                                implicitWidth: 48
                                mdState.size: MD.Enum.S
                                enabled: control.canMutate()
                                inputMethodHints: Qt.ImhDigitsOnly
                                validator: IntValidator { bottom: 0; top: 999 }
                                text: String(control.ivalH())
                                onEditingFinished: {
                                    if (text.length === 0)
                                        text = "0";
                                    control.applyInterval();
                                }
                            }

                            MD.Text {
                                text: "h"
                                typescale: MD.Token.typescale.body_small
                                color: MD.Token.color.on_surface_variant
                            }

                            MD.TextField {
                                id: fieldM

                                implicitWidth: 48
                                mdState.size: MD.Enum.S
                                enabled: control.canMutate()
                                inputMethodHints: Qt.ImhDigitsOnly
                                validator: IntValidator { bottom: 0; top: 59 }
                                text: String(control.ivalM())
                                onEditingFinished: {
                                    if (text.length === 0)
                                        text = "0";
                                    control.applyInterval();
                                }
                            }

                            MD.Text {
                                text: "m"
                                typescale: MD.Token.typescale.body_small
                                color: MD.Token.color.on_surface_variant
                            }

                            MD.TextField {
                                id: fieldS

                                implicitWidth: 48
                                mdState.size: MD.Enum.S
                                enabled: control.canMutate()
                                inputMethodHints: Qt.ImhDigitsOnly
                                validator: IntValidator { bottom: 0; top: 59 }
                                text: String(control.ivalS())
                                onEditingFinished: {
                                    if (text.length === 0)
                                        text = "0";
                                    control.applyInterval();
                                }
                            }

                            MD.Text {
                                text: "s"
                                typescale: MD.Token.typescale.body_small
                                color: MD.Token.color.on_surface_variant
                            }

                            Item { Layout.fillWidth: true }
                        }
                    }
                }
            }
        }
    }
}

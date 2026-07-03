pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import waywallen.ui as W
import Qcm.Material as MD

MD.Dialog {
    id: root
    title: qsTr("Filters")
    horizontalPadding: 16
    implicitWidth: Math.min(440, parent ? parent.width - 48 : 440)
    standardButtons: T.Dialog.Close

    property var availableTags: []
    property var selectedTags: []
    property var filterTagDialog: null
    readonly property var popupParent: root.Window.window ? root.Window.window : root.parent

    signal apply(var tags)

    readonly property var filterTags: sanitizeTags(availableTags).filter(t => t !== "Mature")
    readonly property bool hasMatureFilter: sanitizeTags(availableTags).indexOf("Mature") >= 0
    readonly property var selectedFilterTags: collectSelected(filterTags)

    function sanitizeTags(tags) {
        let out = [];
        let seen = {};
        for (const value of tags ?? []) {
            const tag = String(value);
            if (tag.length === 0 || seen[tag] === true)
                continue;
            seen[tag] = true;
            out.push(tag);
        }
        return out;
    }
    function selectedMap(tags) {
        const allowed = {};
        for (const tag of sanitizeTags(availableTags))
            allowed[tag] = true;
        let out = {};
        for (const tag of sanitizeTags(tags)) {
            if (allowed[tag] === true)
                out[tag] = true;
        }
        return out;
    }
    function has(tag) {
        return selectedMap(selectedTags)[tag] === true;
    }
    function collectSelected(tags) {
        const selected = selectedMap(selectedTags);
        let out = [];
        for (const tag of sanitizeTags(tags)) {
            if (selected[tag] === true)
                out.push(tag);
        }
        return out;
    }
    function collect(filterTags, mature) {
        const selected = {};
        for (const tag of sanitizeTags(filterTags))
            selected[tag] = true;
        let out = [];
        for (const tag of sanitizeTags(availableTags)) {
            if (tag === "Mature") {
                if (mature)
                    out.push(tag);
            } else if (selected[tag] === true) {
                out.push(tag);
            }
        }
        return out;
    }
    function openFilterTagDialog() {
        if (filterTagDialog && (filterTagDialog.opened || filterTagDialog.entering || filterTagDialog.closing))
            return;
        filterTagDialog = MD.Util.showPopup(filterTagDialogComponent, {}, root.popupParent);
    }
    function applyFilterTags(tags) {
        root.apply(collect(tags, has("Mature")));
    }
    function setMature(on) {
        root.apply(collect(selectedFilterTags, on));
    }

    Component {
        id: filterTagDialogComponent

        W.TagPickerDialog {
            id: dynamicFilterTagDialog
            allTags: root.filterTags
            selected: root.selectedFilterTags
            onCommit: function (tags) {
                root.applyFilterTags(tags);
            }
            onClosed: {
                if (root.filterTagDialog === dynamicFilterTagDialog)
                    root.filterTagDialog = null;
            }
        }
    }

    contentItem: ColumnLayout {
        spacing: 16

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            visible: root.filterTags.length > 0

            RowLayout {
                Layout.fillWidth: true
                MD.Label {
                    Layout.fillWidth: true
                    text: qsTr("Tags")
                    typescale: MD.Token.typescale.title_medium
                }
                MD.IconButton {
                    icon.name: MD.Token.icon.edit
                    onClicked: root.openFilterTagDialog()
                }
            }

            Flow {
                Layout.fillWidth: true
                visible: root.selectedFilterTags && root.selectedFilterTags.length > 0
                spacing: 6

                Repeater {
                    model: root.selectedFilterTags
                    delegate: W.Tag {
                        required property var modelData
                        text: modelData
                    }
                }
            }

        }

        MD.Divider {
            Layout.fillWidth: true
            visible: root.hasMatureFilter
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            visible: root.hasMatureFilter

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                MD.Label {
                    text: qsTr("Mature content (NSFW)")
                    typescale: MD.Token.typescale.title_small
                }
                MD.Label {
                    text: qsTr("18+ only. Shows mature-tagged wallpapers.")
                    typescale: MD.Token.typescale.body_small
                    color: MD.Token.color.on_surface_variant
                }
            }

            MD.Switch {
                id: m_nsfw
                checked: root.has("Mature")
                onClicked: {
                    if (!root.has("Mature")) {
                        m_nsfw.checked = Qt.binding(() => root.has("Mature"));
                        m_confirm.open();
                    } else {
                        root.setMature(false);
                        m_nsfw.checked = Qt.binding(() => root.has("Mature"));
                    }
                }
            }
        }
    }

    MD.Dialog {
        id: m_confirm
        title: qsTr("Mature content")
        modal: true
        anchors.centerIn: T.Overlay.overlay
        standardButtons: T.Dialog.Cancel | T.Dialog.Ok
        onAccepted: {
            root.setMature(true);
            m_nsfw.checked = Qt.binding(() => root.has("Mature"));
        }
        onRejected: m_nsfw.checked = Qt.binding(() => root.has("Mature"))

        contentItem: MD.Label {
            text: qsTr("This shows wallpapers tagged as mature / NSFW. You must be 18 or older to enable this. Continue?")
            wrapMode: Text.WordWrap
        }
    }
}

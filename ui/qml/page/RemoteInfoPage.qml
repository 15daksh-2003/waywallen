pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD

MD.Page {
    id: root
    title: "Remote info"
    scrolling: !infoFlick.atYBeginning

    property var item: null
    property var details: null
    property string sourceName: ""
    property int remoteCapability: 0
    property string remoteHint: ""

    readonly property string formattedSize: formatSize(details?.size)
    readonly property string tagsText: formatList(details?.tags)

    function value(v) {
        return v === undefined || v === null ? "" : String(v);
    }

    function hasText(v) {
        return value(v).length > 0;
    }

    function formatList(v) {
        if (!v || v.length === 0)
            return "";
        const out = [];
        for (let i = 0; i < v.length; ++i)
            out.push(String(v[i]));
        return out.join(", ");
    }

    function formatBytes(bytes) {
        let v = Number(bytes ?? 0);
        if (!(v > 0))
            return "";
        const u = ["B", "KB", "MB", "GB", "TB"];
        let i = 0;
        while (v >= 1024 && i < u.length - 1) {
            v /= 1024;
            ++i;
        }
        return v.toFixed(i === 0 ? 0 : 1) + " " + u[i];
    }

    function formatSize(s) {
        const text = String(s ?? "").trim();
        if (text.length === 0)
            return "";
        if (/^\d+$/.test(text))
            return formatBytes(Number(text));
        const m = text.match(/^([\d.,]+)\s*([KMGT]?B)$/i);
        if (!m)
            return text;
        const num = parseFloat(m[1].replace(/,/g, ""));
        if (isNaN(num))
            return text;
        const unit = m[2].toUpperCase();
        if (unit === "B")
            return formatBytes(num);
        return num.toFixed(1) + " " + unit;
    }

    function subscriptionText(state) {
        switch (Number(state ?? 0)) {
        case 1: return qsTr("Unsubscribed");
        case 2: return qsTr("Subscribed");
        case 3: return qsTr("Updating");
        default: return qsTr("Unknown");
        }
    }

    component InfoLabel: MD.Text {
        required property string label

        Layout.preferredWidth: 104
        Layout.alignment: Qt.AlignTop
        text: label
        typescale: MD.Token.typescale.label_medium
        color: MD.Token.color.on_surface_variant
        elide: Text.ElideRight
        maximumLineCount: 1
    }

    component InfoValue: MD.TextEdit {
        Layout.fillWidth: true
        Layout.preferredHeight: Math.max(24, contentHeight)
        readOnly: true
        selectByMouse: true
        persistentSelection: true
        typescale: MD.Token.typescale.body_medium
        color: MD.Token.color.on_surface
        wrapMode: TextEdit.WrapAnywhere
    }

    contentItem: MD.VerticalFlickable {
        id: infoFlick
        topMargin: 12
        bottomMargin: 24
        leftMargin: 16
        rightMargin: 16

        GridLayout {
            width: infoFlick.contentWidth
            columns: 2
            columnSpacing: 12
            rowSpacing: 10

            InfoLabel {
                visible: root.hasText(root.sourceName)
                label: "Source"
            }
            InfoValue {
                visible: root.hasText(root.sourceName)
                text: root.sourceName
            }

            InfoLabel { label: "Source ID" }
            InfoValue { text: root.value(root.item?.sourceId) }

            InfoLabel { label: "Item ID" }
            InfoValue { text: root.value(root.item?.itemId) }

            InfoLabel { label: "Title" }
            InfoValue { text: root.value(root.item?.title) }

            InfoLabel {
                visible: root.hasText(root.item?.wpType)
                label: "Type"
            }
            InfoValue {
                visible: root.hasText(root.item?.wpType)
                text: root.value(root.item?.wpType)
            }

            InfoLabel {
                visible: root.hasText(root.item?.author)
                label: "Author"
            }
            InfoValue {
                visible: root.hasText(root.item?.author)
                text: root.value(root.item?.author)
            }

            InfoLabel {
                visible: root.hasText(root.item?.previewUrl)
                label: "Preview"
            }
            InfoValue {
                visible: root.hasText(root.item?.previewUrl)
                text: root.value(root.item?.previewUrl)
            }

            InfoLabel {
                visible: root.hasText(root.formattedSize)
                label: "Size"
            }
            InfoValue {
                visible: root.hasText(root.formattedSize)
                text: root.formattedSize
            }

            InfoLabel {
                visible: Number(root.details?.width ?? 0) > 0
                label: "Width"
            }
            InfoValue {
                visible: Number(root.details?.width ?? 0) > 0
                text: String(root.details?.width ?? 0)
            }

            InfoLabel {
                visible: Number(root.details?.height ?? 0) > 0
                label: "Height"
            }
            InfoValue {
                visible: Number(root.details?.height ?? 0) > 0
                text: String(root.details?.height ?? 0)
            }

            InfoLabel {
                visible: root.remoteCapability === 1
                label: "Downloaded"
            }
            InfoValue {
                visible: root.remoteCapability === 1
                text: Number(root.item?.acquisitionState ?? 0) === 3 ? "true" : "false"
            }

            InfoLabel {
                visible: root.remoteCapability === 2
                label: "Subscription"
            }
            InfoValue {
                visible: root.remoteCapability === 2
                text: root.subscriptionText(root.item?.acquisitionState)
            }

            InfoLabel {
                visible: root.hasText(root.remoteHint)
                label: "Acquisition"
            }
            InfoValue {
                visible: root.hasText(root.remoteHint)
                text: root.remoteHint
            }

            InfoLabel {
                visible: root.hasText(root.tagsText)
                label: "Tags"
            }
            InfoValue {
                visible: root.hasText(root.tagsText)
                text: root.tagsText
            }

            InfoLabel {
                visible: root.hasText(root.details?.description)
                label: "Description"
            }
            InfoValue {
                visible: root.hasText(root.details?.description)
                text: root.value(root.details?.description)
            }
        }
    }
}

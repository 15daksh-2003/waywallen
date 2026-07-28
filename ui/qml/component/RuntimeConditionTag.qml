pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD

Rectangle {
    id: root

    required property var condition

    readonly property string kind: condition && condition.kind ? condition.kind : "issue"
    readonly property string reason: condition && condition.reason ? condition.reason : ""
    readonly property string label: {
        if (kind === "loading") return qsTr("Loading");
        if (kind === "waiting") return qsTr("Waiting");
        if (kind === "hang") return qsTr("Hang");
        return qsTr("Issue");
    }
    readonly property string detail: {
        if (reason === "first_frame") return qsTr("Waiting for the first frame");
        if (reason === "consumer_release") return qsTr("Waiting for a display to release a frame");
        if (reason === "frame_progress") return qsTr("Renderer stopped producing frames");
        if (reason === "generation_poisoned") return qsTr("Display release state could not be recovered");
        return reason.length > 0 ? qsTr("Runtime issue: %1").arg(reason) : qsTr("Runtime issue");
    }
    readonly property string tooltipText: {
        const displayId = condition ? Number(condition.relatedDisplayId || 0) : 0;
        return displayId > 0 ? detail + qsTr("\nRelated display: #%1").arg(displayId) : detail;
    }

    implicitWidth: content.implicitWidth + 14
    implicitHeight: content.implicitHeight + 6
    radius: height / 2
    color: {
        if (kind === "hang") return MD.Token.color.error_container;
        if (kind === "waiting") return MD.Token.color.tertiary_container;
        return MD.Token.color.secondary_container;
    }

    RowLayout {
        id: content
        anchors.centerIn: parent
        spacing: 4

        MD.Icon {
            name: {
                if (root.kind === "loading") return MD.Token.icon.sync;
                if (root.kind === "waiting") return MD.Token.icon.schedule;
                return MD.Token.icon.warning;
            }
            size: 13
            color: label.color
        }

        MD.Text {
            id: label
            text: root.label
            typescale: MD.Token.typescale.label_small
            color: root.kind === "hang"
                ? MD.Token.color.on_error_container
                : root.kind === "waiting"
                    ? MD.Token.color.on_tertiary_container
                    : MD.Token.color.on_secondary_container
        }
    }

    HoverHandler {
        id: hover
    }

    MD.ToolTip {
        visible: hover.hovered
        delay: 300
        text: root.tooltipText
    }
}

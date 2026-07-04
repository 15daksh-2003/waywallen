pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qcm.Material as MD
import waywallen.ui as W

ColumnLayout {
    id: root

    readonly property string githubUrl: "https://github.com/waywallen/waywallen-display"
    readonly property string desktopDisplayName: displayName(W.Notify.displayBackend.desktop)
    readonly property bool binaryMissing: W.Notify.displayBackend.state === "binary_missing"
    readonly property bool flatpakRestricted: W.Notify.displayBackend.state === "flatpak_restricted"
    readonly property string flatpakLabel: W.Notify.displayBackend.flatpakId.length > 0 ? " (" + W.Notify.displayBackend.flatpakId + ")" : ""

    function displayName(value) {
        const raw = (value || "").trim();
        if (!raw.length)
            return qsTr("This desktop");
        const key = raw.toLowerCase();
        if (key === "cosmic")
            return "COSMIC";
        if (key === "hyprland")
            return "Hyprland";
        if (key === "niri")
            return "Niri";
        if (key === "river")
            return "River";
        if (key === "sway")
            return "Sway";
        return raw.charAt(0).toUpperCase() + raw.slice(1);
    }

    spacing: 12
    visible: W.Notify.displayBackend.name === "layer-shell" && (binaryMissing || flatpakRestricted)

    MD.Text {
        Layout.fillWidth: true
        text: root.flatpakRestricted
            ? qsTr("%1 uses the <b>waywallen-layer-shell</b> display backend. The daemon is running inside Flatpak%2, where layer-shell Wayland protocols are not available. Start and keep <b>waywallen-layer-shell</b> running outside Flatpak.").arg(root.desktopDisplayName).arg(root.flatpakLabel)
            : qsTr("%1 uses the <b>waywallen-layer-shell</b> display backend, but the daemon could not find its binary. Install it from GitHub:").arg(root.desktopDisplayName)
        textFormat: Text.StyledText
        wrapMode: Text.WordWrap
        horizontalAlignment: Text.AlignHCenter
        typescale: MD.Token.typescale.body_medium
        color: MD.Token.color.on_surface
    }

    MD.Button {
        Layout.alignment: Qt.AlignHCenter
        text: qsTr("GitHub")
        mdState.type: MD.Enum.BtFilledTonal
        onClicked: MD.Util.openUrlExternally(root.githubUrl)

        MD.ToolTip {
            visible: parent.hovered
            text: root.githubUrl
        }
    }

    MD.Text {
        Layout.fillWidth: true
        text: root.flatpakRestricted
            ? qsTr("Install <tt>waywallen-layer-shell</tt> from GitHub and start it on the host. Example: <tt>waywallen-layer-shell --socket $XDG_RUNTIME_DIR/waywallen/display.sock</tt>")
            : qsTr("Put a binary named <tt>waywallen-layer-shell</tt> in <tt>PATH</tt> or next to the <tt>waywallen</tt> daemon, then restart waywallen. Manual test: <tt>waywallen-layer-shell --socket $XDG_RUNTIME_DIR/waywallen/display.sock</tt>")
        textFormat: Text.StyledText
        wrapMode: Text.WordWrap
        horizontalAlignment: Text.AlignHCenter
        typescale: MD.Token.typescale.body_small
        color: MD.Token.color.on_surface_variant
    }
}

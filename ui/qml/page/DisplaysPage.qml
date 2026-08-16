pragma ComponentBehavior: Bound
pragma ValueTypeBehavior: Assertable
import QtQuick
import QtQuick.Layouts
import QtQuick.Shapes
import QtQuick.Templates as T
import Qcm.Material as MD
import waywallen.ui as W

MD.Page {
    id: root

    title: qsTr('Displays')
    showHeader: MD.MProp.size.isCompact
    showBackground: false
    readonly property real displayGapPx: 80
    readonly property real canvasHeaderPx: 36
    readonly property real canvasPaddingPx: 8
    readonly property real canvasSnapPx: 12

    property string selectedKind: ""
    property var selectedId: null
    property string pendingCanvasId: ""
    property bool paneAnimationsEnabled: false
    readonly property bool detailsVisible: !!root.selectedDisplayObject || !!root.selectedCanvasObject
    readonly property real paneSpacing: 12
    readonly property real paneAvailableHeight: Math.max(0, height - paneSpacing - (detailsVisible ? paneSpacing / 2 : 0))
    readonly property real displayPaneHeight: detailsVisible ? paneAvailableHeight / 2 : paneAvailableHeight
    readonly property real detailPaneHeight: detailsVisible ? paneAvailableHeight - displayPaneHeight : 0

    // FillMode/Rotation enum values mirror proto::FillMode /
    // proto::Rotation (control.proto). Keep the *_VALUES
    // arrays in lockstep with the enum order; *_LABELS is what the UI
    // shows.
    readonly property var kFillModeValues: [1 // STRETCHED
        , 2 // PRESERVE_ASPECT_FIT
        , 3 // PRESERVE_ASPECT_CROP
        , 7  // CENTERED
    ]
    readonly property var kFillModeLabels: [qsTr("Stretch"), qsTr("Fit"), qsTr("Crop"), qsTr("Center")]
    function fillmodeIndex(value) {
        const i = root.kFillModeValues.indexOf(value);
        return i < 0 ? 0 : i;
    }

    // Rotation segmented values, mirror proto::Rotation:
    //   1=NORMAL, 2=CW_90, 3=CW_180, 4=CW_270
    readonly property var kRotationValues: [1, 2, 3, 4]
    readonly property var kRotationLabels: ["0°", "90°", "180°", "270°"]
    function rotationIndex(value) {
        const i = root.kRotationValues.indexOf(value);
        return i < 0 ? 0 : i;
    }

    function clampPercent(value) {
        return Math.max(0, Math.min(100, Math.round(Number(value) || 0)));
    }

    function prepareCanvasLayoutUpdate() {
        canvasLayoutSetQuery.canvasId = root.selectedCanvasObject.id;
        canvasLayoutSetQuery.fillmodeSet = false;
        canvasLayoutSetQuery.locationSet = false;
        canvasLayoutSetQuery.rotationSet = false;
        canvasLayoutSetQuery.clearFillmode = false;
        canvasLayoutSetQuery.clearLocation = false;
        canvasLayoutSetQuery.clearRotation = false;
    }

    function applyLocation(x, y) {
        if (!root.selected)
            return;
        if (root.selectedKind === "canvas") {
            root.prepareCanvasLayoutUpdate();
            canvasLayoutSetQuery.locationSet = true;
            canvasLayoutSetQuery.locationX = root.clampPercent(x);
            canvasLayoutSetQuery.locationY = root.clampPercent(y);
            canvasLayoutSetQuery.reload();
            return;
        }
        layoutSetQuery.name = root.selected.name;
        layoutSetQuery.displayId = root.selected.id;
        layoutSetQuery.fillmodeSet = false;
        layoutSetQuery.locationSet = true;
        layoutSetQuery.locationX = root.clampPercent(x);
        layoutSetQuery.locationY = root.clampPercent(y);
        layoutSetQuery.alignSet = false;
        layoutSetQuery.rotationSet = false;
        layoutSetQuery.clearFillmode = false;
        layoutSetQuery.clearLocation = false;
        layoutSetQuery.clearAlign = false;
        layoutSetQuery.clearRotation = false;
        layoutSetQuery.reload();
    }

    function applyFillmode(value) {
        if (!root.selected)
            return;
        if (root.selectedKind === "canvas") {
            root.prepareCanvasLayoutUpdate();
            canvasLayoutSetQuery.fillmodeSet = true;
            canvasLayoutSetQuery.fillmode = value;
            canvasLayoutSetQuery.reload();
            return;
        }
        layoutSetQuery.name = root.selected.name;
        layoutSetQuery.displayId = root.selected.id;
        layoutSetQuery.fillmodeSet = true;
        layoutSetQuery.fillmode = value;
        layoutSetQuery.locationSet = false;
        layoutSetQuery.alignSet = false;
        layoutSetQuery.rotationSet = false;
        layoutSetQuery.clearFillmode = false;
        layoutSetQuery.clearLocation = false;
        layoutSetQuery.clearAlign = false;
        layoutSetQuery.clearRotation = false;
        layoutSetQuery.reload();
    }

    function applyRotation(value) {
        if (!root.selected)
            return;
        if (root.selectedKind === "canvas") {
            root.prepareCanvasLayoutUpdate();
            canvasLayoutSetQuery.rotationSet = true;
            canvasLayoutSetQuery.rotation = value;
            canvasLayoutSetQuery.reload();
            return;
        }
        layoutSetQuery.name = root.selected.name;
        layoutSetQuery.displayId = root.selected.id;
        layoutSetQuery.fillmodeSet = false;
        layoutSetQuery.locationSet = false;
        layoutSetQuery.alignSet = false;
        layoutSetQuery.rotationSet = true;
        layoutSetQuery.rotation = value;
        layoutSetQuery.clearFillmode = false;
        layoutSetQuery.clearLocation = false;
        layoutSetQuery.clearAlign = false;
        layoutSetQuery.clearRotation = false;
        layoutSetQuery.reload();
    }

    function resetLayout() {
        if (!root.selected)
            return;
        if (root.selectedKind === "canvas") {
            root.prepareCanvasLayoutUpdate();
            canvasLayoutSetQuery.clearFillmode = true;
            canvasLayoutSetQuery.clearLocation = true;
            canvasLayoutSetQuery.clearRotation = true;
            canvasLayoutSetQuery.reload();
            return;
        }
        layoutSetQuery.name = root.selected.name;
        layoutSetQuery.displayId = root.selected.id;
        layoutSetQuery.fillmodeSet = false;
        layoutSetQuery.locationSet = false;
        layoutSetQuery.alignSet = false;
        layoutSetQuery.clearFillmode = true;
        layoutSetQuery.clearLocation = true;
        layoutSetQuery.clearAlign = true;
        layoutSetQuery.clearRotation = true;
        layoutSetQuery.reload();
    }

    W.DisplayLayoutSetQuery {
        id: layoutSetQuery
    }

    W.CanvasLayoutSetQuery {
        id: canvasLayoutSetQuery
    }

    W.DisplayRenameQuery {
        id: renameQuery
    }

    W.CanvasMutationQuery {
        id: canvasMutationQuery
        forwardError: false
        onCanvasCreated: canvasId => {
            root.pendingCanvasId = canvasId;
            root.openPendingCanvas();
        }
        onCanvasUpdated: function (canvasId, revision) {
            if (root.selectedKind === "canvas" && root.selectedId === canvasId)
                canvasEditor.accept(revision);
        }
        onCanvasDeleted: canvasId => {
            if (root.selectedKind !== "canvas" || root.selectedId !== canvasId)
                return;
            canvasEditor.clear();
            root.selectedKind = "";
            root.selectedId = null;
        }
    }

    W.CanvasEditorState {
        id: canvasEditor
        displays: W.App.displayManager.displays
    }

    Connections {
        target: W.App.displayManager
        function onCanvasesChanged() {
            root.openPendingCanvas();
            if (root.selectedKind !== "canvas")
                return;
            const selected = root.findSelectedCanvas();
            if (!selected) {
                canvasEditor.clear();
                root.selectedKind = "";
                root.selectedId = null;
            } else if (!canvasEditor.dirty) {
                canvasEditor.begin(selected);
            } else {
                canvasEditor.refreshMemberSizes(selected);
            }
        }
    }

    Connections {
        target: canvasMutationQuery
        function onStatusChanged() {
            if (canvasMutationQuery.status === 3)
                W.Global.toastError(canvasMutationQuery.error || qsTr("Canvas update failed"));
        }
    }

    W.DisplayEditDialog {
        id: displayEditDialog
        onSubmitted: function (name, targetId, alias, clear) {
            renameQuery.name = name;
            renameQuery.displayId = targetId;
            renameQuery.alias = alias;
            renameQuery.clear = clear;
            renameQuery.reload();
        }
    }

    W.CanvasDialog {
        id: canvasDialog
        onSubmitted: function (name) {
            if (!root.selectedCanvasObject)
                return;
            canvasMutationQuery.updateCanvas(root.selectedCanvasObject.id, canvasEditor.baseRevision, name, canvasEditor.members);
        }
    }

    MD.Action {
        id: resetCanvasAlignmentAction
        text: qsTr("Reset")
        icon.name: MD.Token.icon.restart_alt
        enabled: canvasEditor.dirty && !canvasMutationQuery.querying
        onTriggered: canvasEditor.cancel()
    }

    MD.Action {
        id: applyCanvasAlignmentAction
        text: qsTr("Apply")
        icon.name: MD.Token.icon.check
        enabled: canvasEditor.dirty && canvasEditor.name.trim().length > 0 && !canvasMutationQuery.querying
        onTriggered: root.applyCanvasDraft()
    }

    MD.Dialog {
        id: deleteCanvasDialog
        parent: T.Overlay.overlay
        modal: true
        title: qsTr("Delete canvas?")
        standardButtons: T.Dialog.Cancel | T.Dialog.Ok
        contentItem: MD.Text {
            text: qsTr("The canvas layout will be removed. Its displays become independent again.")
            wrapMode: Text.Wrap
            color: MD.Token.color.on_surface_variant
        }
        onAccepted: {
            if (root.selectedCanvasObject) {
                canvasMutationQuery.removeCanvas(root.selectedCanvasObject.id, canvasEditor.baseRevision);
            }
        }
    }

    function canvasRows(canvasObject) {
        return canvasEditor.rowsForCanvas(canvasObject);
    }

    function defaultCanvasExtent() {
        let smallest = null;
        for (const display of W.App.displayManager.displays || []) {
            const width = Number(display.width || 0);
            const height = Number(display.height || 0);
            if (width <= 0 || height <= 0)
                continue;
            if (!smallest || width * height < smallest.width * smallest.height)
                smallest = {
                    width: width,
                    height: height
                };
        }
        return smallest || {
            width: 1280,
            height: 720
        };
    }

    function layoutTargets() {
        const out = [];
        const assignedKeys = new Set();
        const defaultExtent = root.defaultCanvasExtent();
        let fallbackX = 0;
        for (const canvasObject of W.App.displayManager.canvases || []) {
            const members = root.canvasRows(canvasObject);
            const bounds = canvasEditor.topologyBounds(members);
            const empty = members.length === 0;
            const width = empty ? defaultExtent.width : bounds.width;
            const height = empty ? defaultExtent.height : bounds.height;
            for (const member of members)
                assignedKeys.add(String(member.settingsKey || ""));
            out.push({
                kind: "canvas",
                x: fallbackX,
                y: 0,
                w: width,
                h: height,
                canvasObject: canvasObject,
                members: members,
                memberBounds: bounds
            });
            fallbackX += width + root.displayGapPx;
        }
        for (const d of W.App.displayManager.displays || []) {
            if (assignedKeys.has(String(d.settingsKey || d.name || "")))
                continue;
            const w = Number(d.width || 1);
            const h = Number(d.height || 1);
            out.push({
                kind: "display",
                x: fallbackX,
                y: 0,
                w: w,
                h: h,
                placed: true,
                displayObject: d
            });
            fallbackX += w + root.displayGapPx;
        }
        return out;
    }

    function openPendingCanvas() {
        if (!root.pendingCanvasId.length)
            return;
        const canvas = W.App.displayManager.getCanvas(root.pendingCanvasId);
        if (!canvas)
            return;
        root.pendingCanvasId = "";
        root.selectCanvas(canvas);
    }

    readonly property var targets: layoutTargets()

    readonly property real boundsW: {
        if (targets.length === 0)
            return 1;
        let min = targets[0].x;
        let max = targets[0].x + targets[0].w;
        for (const r of targets)
            min = Math.min(min, r.x);
        for (const r of targets)
            max = Math.max(max, r.x + r.w);
        return max - min || 1;
    }
    readonly property real boundsH: {
        if (targets.length === 0)
            return 1;
        let min = targets[0].y;
        let max = targets[0].y + targets[0].h;
        for (const r of targets)
            min = Math.min(min, r.y);
        for (const r of targets)
            max = Math.max(max, r.y + r.h);
        return max - min || 1;
    }
    readonly property real boundsX: targets.length > 0 ? targets.reduce((value, item) => Math.min(value, item.x), targets[0].x) : 0
    readonly property real boundsY: targets.length > 0 ? targets.reduce((value, item) => Math.min(value, item.y), targets[0].y) : 0

    function findSelectedDisplay() {
        if (root.selectedKind !== "display" || root.selectedId === null)
            return null;
        for (const d of W.App.displayManager.displays || []) {
            if (d.id === root.selectedId)
                return d;
        }
        return null;
    }

    function findSelectedCanvas() {
        if (root.selectedKind !== "canvas" || root.selectedId === null)
            return null;
        return W.App.displayManager.getCanvas(String(root.selectedId));
    }

    function canChangeSelection(kind, id) {
        return !canvasEditor.dirty || (root.selectedKind === kind && root.selectedId === id);
    }

    function selectDisplay(displayObject) {
        if (!displayObject || !root.canChangeSelection("display", displayObject.id))
            return;
        if (root.selectedKind === "display" && root.selectedId === displayObject.id) {
            root.clearSelection();
            return;
        }
        canvasEditor.clear();
        root.selectedKind = "display";
        root.selectedId = displayObject.id;
    }

    function selectCanvas(canvasObject) {
        if (!canvasObject || !root.canChangeSelection("canvas", canvasObject.id))
            return;
        if (root.selectedKind !== "canvas" || root.selectedId !== canvasObject.id) {
            root.selectedKind = "canvas";
            root.selectedId = canvasObject.id;
            canvasEditor.begin(canvasObject);
        }
    }

    function clearSelection() {
        if (canvasEditor.dirty)
            return;
        canvasEditor.clear();
        root.selectedKind = "";
        root.selectedId = null;
    }

    function applyCanvasDraft() {
        if (!root.selectedCanvasObject || !canvasEditor.dirty)
            return;
        canvasMutationQuery.updateCanvas(root.selectedCanvasObject.id, canvasEditor.baseRevision, canvasEditor.name.trim(), canvasEditor.members);
    }

    function dropDisplayOnCanvas(canvasObject, displayObject, dropX, dropY, memberBounds) {
        if (!canvasObject || !displayObject || !root.canChangeSelection("canvas", canvasObject.id))
            return false;
        root.selectCanvas(canvasObject);
        if (root.selectedKind !== "canvas" || root.selectedId !== canvasObject.id)
            return false;
        const bounds = memberBounds || ({
                x: 0,
                y: 0
            });
        canvasEditor.addDisplayAtEdge(displayObject, Number(bounds.x || 0) + (dropX - root.canvasPaddingPx) / canvas.viewScale);
        return true;
    }

    function dragPayload(source) {
        return {
            kind: source?.dragKind || "",
            displayObject: source?.displayObject || null
        };
    }

    readonly property var selectedDisplayObject: findSelectedDisplay()
    readonly property var selectedCanvasObject: findSelectedCanvas()
    readonly property var selected: selectedDisplayObject || selectedCanvasObject

    Item {
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.rightMargin: 12

        Timer {
            interval: 0
            running: true
            repeat: false
            onTriggered: root.paneAnimationsEnabled = true
        }

        MD.Pane {
            id: displaysPane
            x: 0
            y: root.paneSpacing / 2
            width: parent.width
            height: root.displayPaneHeight
            horizontalPadding: 24
            verticalPadding: 16
            radius: 16
            backgroundColor: MD.MProp.color.surface

            Behavior on height {
                enabled: root.paneAnimationsEnabled

                NumberAnimation {
                    duration: 200
                    easing.type: Easing.InOutCubic
                }
            }

            contentItem: Item {
                id: canvas
                implicitHeight: 48

                Item {
                    id: canvasActions
                    anchors.right: parent.right
                    anchors.top: parent.top
                    width: createCanvasChip.implicitWidth + refreshDisplaysButton.implicitWidth + 6 + (canvasAlignmentActionToolBar.visible ? canvasAlignmentActionToolBar.implicitWidth + 6 : 0)
                    height: createCanvasChip.implicitHeight
                    z: 100

                    MD.ActionToolBar {
                        id: canvasAlignmentActionToolBar

                        anchors.right: refreshDisplaysButton.left
                        anchors.rightMargin: 6
                        anchors.verticalCenter: parent.verticalCenter
                        visible: canvasEditor.dirty
                        actions: [resetCanvasAlignmentAction, applyCanvasAlignmentAction]
                        iconDelegate: MD.SmallIconButton {
                            id: canvasActionButton

                            readonly property string toolTipText: canvasActionButton.action?.tooltip || canvasActionButton.action?.text || ""

                            action: MD.ToolBarLayout.action
                            hoverEnabled: true
                            MD.ToolTip.text: toolTipText
                            MD.ToolTip.visible: hovered && toolTipText.length > 0 && !pressed
                        }
                        moreDelegate: MD.SmallIconButton {
                            action: canvasAlignmentActionToolBar.moreAction
                        }
                    }

                    MD.SmallIconButton {
                        id: refreshDisplaysButton

                        anchors.right: createCanvasChip.left
                        anchors.rightMargin: 6
                        anchors.verticalCenter: parent.verticalCenter
                        icon.name: MD.Token.icon.refresh
                        enabled: W.DaemonDBusClient.daemonAvailable
                        hoverEnabled: true
                        MD.ToolTip.text: qsTr("Refresh displays")
                        MD.ToolTip.visible: hovered && !pressed
                        onClicked: {
                            if (!W.DaemonDBusClient.refreshDisplays())
                                W.Global.toastError(qsTr("Failed to refresh displays"));
                        }
                    }

                    MD.AssistChip {
                        id: createCanvasChip

                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("Create canvas")
                        icon.name: MD.Token.icon.add
                        enabled: !canvasMutationQuery.querying
                        onClicked: canvasMutationQuery.createCanvas(qsTr("Canvas %1").arg((W.App.displayManager.canvases || []).length + 1), [])
                    }
                }

                readonly property bool hasCanvasTargets: root.targets.some(target => target.kind === "canvas")
                readonly property bool hasHorizontalCanvasTargets: root.targets.some(target => target.kind === "canvas" && target.w >= target.h)
                readonly property bool hasVerticalCanvasTargets: root.targets.some(target => target.kind === "canvas" && target.h > target.w)
                readonly property real drawingPadding: hasCanvasTargets ? root.canvasPaddingPx : 0
                readonly property real drawingLeft: (hasVerticalCanvasTargets ? root.canvasHeaderPx : 0) + drawingPadding
                readonly property real drawingTop: canvasActions.height + 12 + (hasHorizontalCanvasTargets ? root.canvasHeaderPx : 0) + drawingPadding
                readonly property real drawingWidth: Math.max(1, width - drawingLeft - drawingPadding)
                readonly property real drawingHeight: Math.max(1, height - drawingTop - drawingPadding)
                readonly property real viewScale: {
                    return Math.min(drawingWidth / root.boundsW, drawingHeight / root.boundsH);
                }
                readonly property real offsetX: drawingLeft + (drawingWidth - root.boundsW * viewScale) / 2
                readonly property real offsetY: drawingTop + (drawingHeight - root.boundsH * viewScale) / 2

                MouseArea {
                    anchors.fill: parent
                    onClicked: root.clearSelection()
                }

                ColumnLayout {
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: canvas.drawingTop + (canvas.drawingHeight - height) / 2
                    width: Math.min(parent.width - 64, 480)
                    spacing: 12
                    visible: root.targets.length === 0

                    MD.Text {
                        Layout.alignment: Qt.AlignHCenter
                        text: qsTr("No displays registered")
                        typescale: MD.Token.typescale.title_medium
                        color: MD.Token.color.on_surface_variant
                    }

                    // Desktop-specific install hints are self-gated on
                    // `W.Util.desktop`, so this section stays empty when
                    // the daemon can spawn its own display backend.
                    W.KdeDisplaysHelp {
                        Layout.fillWidth: true
                    }

                    W.GnomeDisplaysHelp {
                        Layout.fillWidth: true
                    }

                    W.LayerShellDisplaysHelp {
                        Layout.fillWidth: true
                    }
                }

                Repeater {
                    model: root.targets

                    delegate: Item {
                        id: targetItem
                        required property int index
                        required property var modelData

                        readonly property bool isDisplay: modelData.kind === "display"
                        readonly property bool isCanvas: modelData.kind === "canvas"
                        readonly property var d: modelData.displayObject || null
                        readonly property var canvasObject: modelData.canvasObject || null
                        readonly property var canvasMembers: isCanvas ? (modelData.members || []) : []
                        readonly property var canvasMemberBounds: isCanvas ? (modelData.memberBounds || ({
                                    x: 0,
                                    y: 0,
                                    width: 1,
                                    height: 1
                                })) : ({
                                x: 0,
                                y: 0,
                                width: 1,
                                height: 1
                            })
                        readonly property bool hasLink: isDisplay ? !!d && d.links && d.links.length > 0 : isCanvas && !!canvasObject && canvasObject.links && canvasObject.links.length > 0
                        readonly property bool isSelected: isDisplay ? root.selectedKind === "display" && root.selectedId === d?.id : root.selectedKind === "canvas" && root.selectedId === canvasObject?.id
                        readonly property bool verticalCanvasHeader: isCanvas && modelData.h > modelData.w
                        readonly property real bodyY: canvas.offsetY + (modelData.y - root.boundsY) * canvas.viewScale
                        readonly property real canvasPadding: isCanvas ? root.canvasPaddingPx : 0
                        readonly property real bodyX: canvas.offsetX + (modelData.x - root.boundsX) * canvas.viewScale

                        z: isDisplay ? 2 : 1
                        x: isCanvas ? bodyX - (verticalCanvasHeader ? root.canvasHeaderPx : 0) - canvasPadding : bodyX
                        y: isCanvas ? bodyY - (verticalCanvasHeader ? 0 : root.canvasHeaderPx) - canvasPadding : bodyY
                        width: modelData.w * canvas.viewScale + (verticalCanvasHeader ? root.canvasHeaderPx : 0) + canvasPadding * 2
                        height: modelData.h * canvas.viewScale + (isCanvas && !verticalCanvasHeader ? root.canvasHeaderPx : 0) + canvasPadding * 2

                        Item {
                            id: displayCard
                            property string dragKind: "display"
                            property var displayObject: targetItem.d
                            property bool dragging: false

                            visible: targetItem.isDisplay
                            width: targetItem.width
                            height: targetItem.height
                            z: dragging ? 100 : 0

                            Drag.active: dragging
                            Drag.source: displayCard
                            Drag.keys: ["waywallen/display"]
                            Drag.hotSpot.x: width / 2
                            Drag.hotSpot.y: height / 2
                            Drag.supportedActions: Qt.MoveAction
                            Drag.proposedAction: Qt.MoveAction

                            DragHandler {
                                id: displayDrag
                                onActiveChanged: {
                                    if (active) {
                                        displayCard.dragging = true;
                                    } else if (displayCard.dragging) {
                                        displayCard.Drag.drop();
                                        displayCard.dragging = false;
                                    }
                                }
                            }

                            states: State {
                                when: displayCard.dragging
                                ParentChange {
                                    target: displayCard
                                    parent: canvas
                                }
                            }

                            Shape {
                                visible: targetItem.isDisplay
                                anchors.fill: parent
                                preferredRendererType: Shape.CurveRenderer
                                antialiasing: true

                                ShapePath {
                                    strokeColor: targetItem.isSelected ? MD.Token.color.primary : (targetItem.modelData.placed ? MD.Token.color.outline : MD.Token.color.error)
                                    strokeWidth: targetItem.isSelected ? 3 : 1.5
                                    fillColor: targetItem.hasLink ? MD.Token.color.primary_container : MD.Token.color.surface_container_highest
                                    capStyle: ShapePath.RoundCap
                                    joinStyle: ShapePath.RoundJoin

                                    PathRectangle {
                                        x: 0
                                        y: 0
                                        width: targetItem.width
                                        height: targetItem.height
                                        radius: 10
                                    }
                                }
                            }

                            MouseArea {
                                visible: targetItem.isDisplay
                                anchors.fill: parent
                                onClicked: root.selectDisplay(targetItem.d)
                            }

                            ColumnLayout {
                                visible: targetItem.isDisplay
                                anchors.centerIn: parent
                                width: Math.max(0, targetItem.width - 12)
                                spacing: 4

                                MD.Text {
                                    Layout.fillWidth: true
                                    text: targetItem.d?.displayLabel || qsTr("Display #%1").arg(targetItem.d?.id)
                                    typescale: MD.Token.typescale.title_small
                                    color: targetItem.hasLink ? MD.Token.color.on_primary_container : MD.Token.color.on_surface
                                    horizontalAlignment: Text.AlignHCenter
                                    maximumLineCount: 2
                                    wrapMode: Text.Wrap
                                    elide: Text.ElideRight
                                }

                                MD.Text {
                                    Layout.alignment: Qt.AlignHCenter
                                    text: (targetItem.d?.width || 0) + " × " + (targetItem.d?.height || 0)
                                    typescale: MD.Token.typescale.label_medium
                                    color: targetItem.hasLink ? MD.Token.color.on_primary_container : MD.Token.color.on_surface_variant
                                }
                            }

                            MD.Text {
                                visible: targetItem.isDisplay
                                anchors.left: parent.left
                                anchors.top: parent.top
                                anchors.margins: 6
                                text: "#" + (targetItem.d?.id || "")
                                typescale: MD.Token.typescale.label_small
                                color: targetItem.hasLink ? MD.Token.color.on_primary_container : MD.Token.color.on_surface_variant
                            }

                            W.GpuTag {
                                visible: targetItem.isDisplay
                                anchors.right: parent.right
                                anchors.top: parent.top
                                anchors.margins: 6
                                drmRenderMajor: targetItem.d?.drmRenderMajor || 0
                                drmRenderMinor: targetItem.d?.drmRenderMinor || 0
                            }

                            Flow {
                                visible: targetItem.isDisplay
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                anchors.margins: 6
                                spacing: 4

                                Repeater {
                                    model: targetItem.d?.runtimeConditions || []
                                    delegate: W.RuntimeConditionTag {
                                        required property var modelData
                                        condition: modelData
                                    }
                                }
                            }
                        }

                        Rectangle {
                            id: canvasFrame
                            visible: targetItem.isCanvas
                            anchors.fill: parent
                            radius: 12
                            color: targetItem.hasLink ? MD.Token.color.secondary_container : MD.Token.color.surface_container_highest
                            border.width: canvasDropArea.containsDrag || targetItem.isSelected ? 3 : 1.5
                            border.color: canvasDropArea.containsDrag || targetItem.isSelected ? MD.Token.color.primary : MD.Token.color.outline
                        }

                        Rectangle {
                            id: canvasTitleBar
                            visible: targetItem.isCanvas
                            x: 0
                            y: 0
                            width: targetItem.verticalCanvasHeader ? root.canvasHeaderPx : parent.width
                            height: targetItem.verticalCanvasHeader ? parent.height : root.canvasHeaderPx
                            color: "transparent"

                            TapHandler {
                                onTapped: root.selectCanvas(targetItem.canvasObject)
                            }

                            RowLayout {
                                id: canvasTitleContent

                                anchors.centerIn: parent
                                width: Math.max(0, (targetItem.verticalCanvasHeader ? parent.height : parent.width) - 20)
                                height: root.canvasHeaderPx
                                rotation: targetItem.verticalCanvasHeader ? -90 : 0
                                spacing: 6

                                MD.Icon {
                                    name: MD.Token.icon.dashboard
                                    size: 18
                                    color: targetItem.hasLink ? MD.Token.color.on_secondary_container : MD.Token.color.on_surface
                                }

                                MD.Text {
                                    Layout.fillWidth: true
                                    Layout.minimumWidth: 0
                                    text: targetItem.canvasObject?.name || qsTr("Unnamed canvas")
                                    typescale: MD.Token.typescale.label_medium
                                    color: targetItem.hasLink ? MD.Token.color.on_secondary_container : MD.Token.color.on_surface
                                    maximumLineCount: 1
                                    wrapMode: Text.NoWrap
                                    elide: Text.ElideRight
                                }

                                MD.Text {
                                    text: targetItem.canvasMembers.length + " · " + targetItem.modelData.w + " × " + targetItem.modelData.h
                                    typescale: MD.Token.typescale.label_small
                                    color: targetItem.hasLink ? MD.Token.color.on_secondary_container : MD.Token.color.on_surface_variant
                                }
                            }

                            Rectangle {
                                x: targetItem.verticalCanvasHeader ? parent.width - 1 : 8
                                y: targetItem.verticalCanvasHeader ? 8 : parent.height - 1
                                width: targetItem.verticalCanvasHeader ? 1 : Math.max(0, parent.width - 16)
                                height: targetItem.verticalCanvasHeader ? Math.max(0, parent.height - 16) : 1
                                color: MD.Token.color.outline_variant
                            }
                        }

                        Rectangle {
                            id: canvasTarget
                            visible: targetItem.isCanvas
                            x: targetItem.verticalCanvasHeader ? root.canvasHeaderPx : 0
                            y: targetItem.verticalCanvasHeader ? 0 : root.canvasHeaderPx
                            width: parent.width - (targetItem.verticalCanvasHeader ? root.canvasHeaderPx : 0)
                            height: parent.height - (targetItem.verticalCanvasHeader ? 0 : root.canvasHeaderPx)
                            color: "transparent"

                            DropArea {
                                id: canvasDropArea
                                anchors.fill: parent
                                keys: ["waywallen/display"]
                                enabled: !canvasMutationQuery.querying && root.canChangeSelection("canvas", targetItem.canvasObject?.id)
                                onDropped: drop => {
                                    const payload = root.dragPayload(drop.source);
                                    if (payload.kind === "display" && root.dropDisplayOnCanvas(targetItem.canvasObject, payload.displayObject, drop.x, drop.y, targetItem.canvasMemberBounds)) {
                                        drop.acceptProposedAction();
                                    } else {
                                        drop.accepted = false;
                                    }
                                }
                            }

                            TapHandler {
                                onTapped: root.selectCanvas(targetItem.canvasObject)
                            }

                            MD.Text {
                                anchors.centerIn: parent
                                visible: targetItem.canvasMembers.length === 0
                                text: qsTr("Empty canvas")
                                typescale: MD.Token.typescale.body_medium
                                color: targetItem.hasLink ? MD.Token.color.on_secondary_container : MD.Token.color.on_surface_variant
                            }

                            Repeater {
                                model: targetItem.canvasMembers

                                delegate: Rectangle {
                                    id: memberItem
                                    required property int index
                                    required property var modelData

                                    property bool dragging: false
                                    property vector2d dragStartTranslation: Qt.vector2d(0, 0)
                                    readonly property vector2d dragTranslation: Qt.vector2d(memberDrag.persistentTranslation.x - dragStartTranslation.x, memberDrag.persistentTranslation.y - dragStartTranslation.y)

                                    readonly property bool selected: targetItem.isSelected && (memberTap.pressed || dragging)
                                    readonly property var dragPosition: {
                                        const x = Number(modelData.x || 0) + (dragging ? dragTranslation.x / canvas.viewScale : 0);
                                        const y = Number(modelData.y || 0) + (dragging ? dragTranslation.y / canvas.viewScale : 0);
                                        if (!dragging)
                                            return {
                                                x: x,
                                                y: y
                                            };
                                        return canvasEditor.snappedMemberPosition(index, x, y, root.canvasSnapPx / canvas.viewScale);
                                    }
                                    x: root.canvasPaddingPx + (dragPosition.x - targetItem.canvasMemberBounds.x) * canvas.viewScale
                                    y: root.canvasPaddingPx + (dragPosition.y - targetItem.canvasMemberBounds.y) * canvas.viewScale
                                    width: modelData.width * canvas.viewScale
                                    height: modelData.height * canvas.viewScale
                                    z: dragging ? 100 : 0
                                    radius: 8
                                    color: selected ? MD.Token.color.primary_container : MD.Token.color.surface_container_highest
                                    border.width: 1
                                    border.color: MD.Token.color.outline_variant

                                    DragHandler {
                                        id: memberDrag
                                        target: null
                                        enabled: root.canChangeSelection("canvas", targetItem.canvasObject?.id)
                                        onActiveChanged: {
                                            if (active) {
                                                memberItem.dragStartTranslation = persistentTranslation;
                                                root.selectCanvas(targetItem.canvasObject);
                                                if (root.selectedKind === "canvas" && root.selectedId === targetItem.canvasObject?.id)
                                                    memberItem.dragging = true;
                                            } else if (memberItem.dragging) {
                                                const pointer = targetItem.mapFromItem(null, centroid.scenePosition.x, centroid.scenePosition.y);
                                                const translation = memberItem.dragTranslation;
                                                const baseX = Number(memberItem.modelData.x || 0);
                                                const baseY = Number(memberItem.modelData.y || 0);
                                                memberItem.dragging = false;
                                                if (pointer.x < 0 || pointer.y < 0 || pointer.x > targetItem.width || pointer.y > targetItem.height)
                                                    canvasEditor.removeAt(memberItem.index);
                                                else if (translation.x !== 0 || translation.y !== 0)
                                                    canvasEditor.moveTo(memberItem.index, baseX + translation.x / canvas.viewScale, baseY + translation.y / canvas.viewScale, root.canvasSnapPx / canvas.viewScale);
                                            }
                                        }
                                    }

                                    TapHandler {
                                        id: memberTap
                                        enabled: root.canChangeSelection("canvas", targetItem.canvasObject?.id)
                                        onPressedChanged: if (pressed)
                                            root.selectCanvas(targetItem.canvasObject)
                                    }

                                    ColumnLayout {
                                        anchors.centerIn: parent
                                        width: Math.max(0, parent.width - 12)
                                        spacing: 2

                                        MD.Text {
                                            Layout.fillWidth: true
                                            text: memberItem.modelData.label || memberItem.modelData.settingsKey
                                            horizontalAlignment: Text.AlignHCenter
                                            elide: Text.ElideMiddle
                                            typescale: MD.Token.typescale.label_medium
                                            color: memberItem.selected ? MD.Token.color.on_primary_container : MD.Token.color.on_surface
                                        }

                                        MD.Text {
                                            Layout.fillWidth: true
                                            visible: memberItem.modelData.onlineCount !== 1
                                            text: memberItem.modelData.onlineCount > 1 ? qsTr("%1 overlapping displays").arg(memberItem.modelData.onlineCount) : qsTr("Offline")
                                            horizontalAlignment: Text.AlignHCenter
                                            elide: Text.ElideRight
                                            typescale: MD.Token.typescale.label_small
                                            color: memberItem.selected ? MD.Token.color.on_primary_container : MD.Token.color.on_surface_variant
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Details panel ---
        MD.Pane {
            id: detailsPane
            anchors.top: displaysPane.bottom
            anchors.topMargin: root.paneSpacing
            width: parent.width
            height: root.detailPaneHeight
            visible: root.detailsVisible || height > 0.5

            radius: 16
            corners: MD.Util.corners(radius, radius, 0, 0)
            backgroundColor: MD.MProp.color.surface
            clip: true

            Behavior on height {
                enabled: root.paneAnimationsEnabled

                NumberAnimation {
                    duration: 200
                    easing.type: Easing.InOutCubic
                }
            }

            contentItem: MD.Flickable2 {
                id: detailsFlick
                clip: true
                leftMargin: 16
                rightMargin: 16
                contentWidth: Math.max(0, width - leftMargin - rightMargin)
                contentHeight: root.detailsVisible ? detailsContent.implicitHeight : 0
                flickableDirection: MD.Flickable2.VerticalFlick
                interactive: contentHeight > height

                ColumnLayout {
                    id: detailsContent
                    width: detailsFlick.contentWidth
                    spacing: 8
                    visible: root.detailsVisible

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 8
                        visible: !!root.selected

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8

                            readonly property bool canEdit: root.selectedKind === "canvas" || (root.selectedKind === "display" && W.Util.supportsDisplayRename)

                            MD.Text {
                                Layout.fillWidth: true
                                text: root.selected ? (root.selected.displayLabel || qsTr("Display #%1").arg(root.selected.id)) : ""
                                typescale: MD.Token.typescale.title_medium
                                color: MD.Token.color.on_surface
                                elide: Text.ElideRight
                            }

                            Repeater {
                                model: root.selected ? (root.selected.runtimeConditions || []) : []
                                delegate: W.RuntimeConditionTag {
                                    required property var modelData
                                    Layout.alignment: Qt.AlignVCenter
                                    condition: modelData
                                }
                            }

                            MD.IconButton {
                                visible: parent.canEdit && !!root.selected
                                enabled: !renameQuery.querying && !canvasMutationQuery.querying && !canvasEditor.dirty
                                icon.name: MD.Token.icon.edit
                                MD.ToolTip.visible: hovered
                                MD.ToolTip.text: root.selectedKind === "canvas" ? qsTr("Edit canvas") : qsTr("Edit display")
                                onClicked: {
                                    if (root.selectedKind === "canvas") {
                                        canvasDialog.openFor(root.selectedCanvasObject);
                                    } else {
                                        displayEditDialog.openFor(root.selectedDisplayObject);
                                    }
                                }
                            }

                            MD.IconButton {
                                visible: root.selectedKind === "canvas"
                                enabled: !canvasEditor.dirty && !canvasMutationQuery.querying
                                icon.name: MD.Token.icon.delete_forever
                                MD.ToolTip.visible: hovered
                                MD.ToolTip.text: qsTr("Delete canvas")
                                onClicked: deleteCanvasDialog.open()
                            }

                            MD.IconButton {
                                enabled: !canvasEditor.dirty
                                icon.name: MD.Token.icon.close
                                onClicked: root.clearSelection()
                            }
                        }

                        Flow {
                            Layout.fillWidth: true
                            spacing: 24

                            RowLayout {
                                spacing: 8
                                MD.Text {
                                    text: qsTr("ID:")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }
                                MD.Text {
                                    text: root.selected ? (root.selectedKind === "display" ? "#" : "") + root.selected.id : ""
                                    typescale: MD.Token.typescale.body_medium
                                    color: MD.Token.color.on_surface
                                }
                            }

                            RowLayout {
                                spacing: 8
                                MD.Text {
                                    text: qsTr("Size:")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }
                                MD.Text {
                                    readonly property var canvasBounds: canvasEditor.topologyBounds(canvasEditor.members)
                                    text: {
                                        if (!root.selected)
                                            return "";
                                        if (root.selectedKind === "canvas")
                                            return canvasEditor.members.length > 0 ? canvasBounds.width + " × " + canvasBounds.height : qsTr("Empty");
                                        return root.selected.width + " × " + root.selected.height;
                                    }
                                    typescale: MD.Token.typescale.body_medium
                                    color: MD.Token.color.on_surface
                                }
                            }

                            RowLayout {
                                visible: root.selectedKind === "canvas"
                                spacing: 8
                                MD.Text {
                                    text: qsTr("Members:")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }
                                MD.Text {
                                    readonly property int onlineCount: canvasEditor.members.reduce((total, member) => total + Number(member.onlineCount || 0), 0)
                                    text: qsTr("%1 total, %2 online").arg(canvasEditor.members.length).arg(onlineCount)
                                    typescale: MD.Token.typescale.body_medium
                                    color: MD.Token.color.on_surface
                                }
                            }

                            RowLayout {
                                visible: !!root.selected && root.selectedKind === "display" && root.selected.refreshMhz > 0
                                spacing: 8
                                MD.Text {
                                    text: qsTr("Refresh:")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }
                                MD.Text {
                                    text: root.selected ? (root.selected.refreshMhz / 1000).toFixed(3) + " Hz" : ""
                                    typescale: MD.Token.typescale.body_medium
                                    color: MD.Token.color.on_surface
                                }
                            }

                            RowLayout {
                                visible: !!root.selected && root.selectedKind === "display" && (root.selected.canvasId || "").length > 0
                                spacing: 8
                                MD.Text {
                                    text: qsTr("Canvas area:")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }
                                MD.Text {
                                    readonly property var rect: root.selected ? (root.selected.canvasRect || ({})) : ({})
                                    text: Number(rect.width || 0) > 0 ? (rect.x + ", " + rect.y + " · " + rect.width + " × " + rect.height) : ""
                                    typescale: MD.Token.typescale.body_medium
                                    color: MD.Token.color.on_surface
                                }
                            }

                            MD.AssistChip {
                                visible: !!root.selected && root.selectedKind === "display" && (root.selected.canvasId || "").length > 0
                                text: {
                                    if (!root.selected)
                                        return "";
                                    const canvas = W.App.displayManager.getCanvas(root.selected.canvasId || "");
                                    return canvas ? canvas.name : qsTr("Canvas");
                                }
                                onClicked: {
                                    const canvas = W.App.displayManager.getCanvas(root.selected.canvasId || "");
                                    if (canvas)
                                        root.selectCanvas(canvas);
                                }
                            }
                        }

                        MD.Divider {
                            Layout.fillWidth: true
                            Layout.topMargin: 4
                            Layout.bottomMargin: 4
                        }

                        MD.Text {
                            text: connectedRow.active ? qsTr("Connected") : qsTr("Assigned")
                            typescale: MD.Token.typescale.title_small
                            color: MD.Token.color.on_surface
                        }

                        RowLayout {
                            id: connectedRow
                            readonly property string connectedId: {
                                if (!root.selected)
                                    return "";
                                const links = root.selected.links || [];
                                return links.length > 0 ? (links[0].rendererId || "") : "";
                            }
                            readonly property bool active: {
                                if (!root.selected)
                                    return false;
                                const links = root.selected.links || [];
                                return links.length > 0 && !!links[0].active;
                            }
                            // Re-resolve when the manager's renderer list changes
                            // (the `renderers` access wires up the dependency) so a
                            // late RendererUpsert or a RendererRemoved is reflected
                            // without manual refresh.
                            readonly property var renderer: {
                                const _ = W.App.rendererManager.renderers;
                                return connectedId.length > 0 ? W.App.rendererManager.get(connectedId) : null;
                            }
                            readonly property int activePlaylistId: root.selected ? Number(root.selected.activePlaylistId || 0) : 0
                            readonly property var playlistStatus: root.selected ? (root.selected.playlistStatus || ({})) : ({})
                            readonly property bool hasPlaylist: activePlaylistId > 0
                            readonly property string playlistDetail: {
                                const status = playlistStatus || ({});
                                const parts = [];
                                const count = Number(status.count || 0);
                                const position = Number(status.position || 0);
                                const remaining = Number(status.remainingSecs || 0);
                                if (count > 0)
                                    parts.push(Math.min(position + 1, count) + " / " + count);
                                if (remaining > 0)
                                    parts.push(qsTr("%n min left", "", Math.ceil(remaining / 60)));
                                return parts.join(" · ");
                            }
                            Layout.fillWidth: true
                            spacing: 16

                            RowLayout {
                                Layout.fillWidth: true
                                Layout.minimumWidth: 0
                                spacing: 8

                                MD.Icon {
                                    readonly property string status: connectedRow.renderer ? connectedRow.renderer.status : ""
                                    name: {
                                        if (!connectedRow.renderer || !connectedRow.active)
                                            return MD.Token.icon.pause;
                                        return status === "paused" ? MD.Token.icon.pause : MD.Token.icon.play_arrow;
                                    }
                                    size: 24
                                    color: !connectedRow.renderer || !connectedRow.active || status === "paused" ? MD.Token.color.on_surface_variant : MD.Token.color.primary
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    Layout.minimumWidth: 0
                                    spacing: 0

                                    MD.Text {
                                        Layout.fillWidth: true
                                        text: {
                                            const r = connectedRow.renderer;
                                            if (r) {
                                                const name = (r.name && r.name.length) ? r.name : "renderer";
                                                return r.pid > 0 ? (name + "-" + r.pid) : name;
                                            }
                                            if (connectedRow.connectedId.length > 0) {
                                                return connectedRow.connectedId;
                                            }
                                            return qsTr("Idle");
                                        }
                                        typescale: MD.Token.typescale.body_medium
                                        color: connectedRow.renderer ? MD.Token.color.on_surface : MD.Token.color.on_surface_variant
                                        font.family: connectedRow.renderer ? "monospace" : ""
                                        elide: Text.ElideMiddle
                                    }

                                    MD.Text {
                                        Layout.fillWidth: true
                                        visible: !!connectedRow.renderer
                                        text: {
                                            const r = connectedRow.renderer;
                                            if (!r)
                                                return "";
                                            const parts = [(r.status || ""), (r.fps || 0) + " fps"];
                                            const textureWidth = Number(r.textureWidth || 0);
                                            const textureHeight = Number(r.textureHeight || 0);
                                            if (textureWidth > 0 && textureHeight > 0)
                                                parts.push(textureWidth + " × " + textureHeight);
                                            return parts.join(" · ");
                                        }
                                        typescale: MD.Token.typescale.label_small
                                        color: MD.Token.color.on_surface_variant
                                        elide: Text.ElideRight
                                    }
                                }
                            }

                            RowLayout {
                                visible: connectedRow.hasPlaylist
                                Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
                                Layout.maximumWidth: Math.max(220, connectedRow.width * 0.4)
                                spacing: 8

                                MD.Icon {
                                    name: MD.Token.icon.playlist_play
                                    size: 24
                                    color: MD.Token.color.primary
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    Layout.minimumWidth: 0
                                    spacing: 0

                                    MD.Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Playlist #%1").arg(connectedRow.activePlaylistId)
                                        typescale: MD.Token.typescale.body_medium
                                        color: MD.Token.color.on_surface
                                        elide: Text.ElideRight
                                    }

                                    MD.Text {
                                        Layout.fillWidth: true
                                        visible: connectedRow.playlistDetail.length > 0
                                        text: connectedRow.playlistDetail
                                        typescale: MD.Token.typescale.label_small
                                        color: MD.Token.color.on_surface_variant
                                        elide: Text.ElideRight
                                    }
                                }
                            }
                        }

                        // ---- Layout (fillmode + location) ----
                        MD.Divider {
                            Layout.fillWidth: true
                            Layout.topMargin: 8
                            Layout.bottomMargin: 4
                            visible: !!root.selected && (root.selectedKind === "canvas" || !(root.selected.canvasId || "").length)
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            visible: !!root.selected && (root.selectedKind === "canvas" || !(root.selected.canvasId || "").length)
                            spacing: 8

                            MD.Text {
                                Layout.fillWidth: true
                                text: qsTr("Layout")
                                typescale: MD.Token.typescale.title_small
                                color: MD.Token.color.on_surface
                            }

                            MD.AssistChip {
                                visible: !!root.selected && root.selectedKind === "display" && root.selected.layoutOverriddenByWallpaper
                                text: qsTr("Wallpaper override")
                            }

                            Item {
                                implicitWidth: children[0].implicitWidth
                                MD.IconButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    mdState.size: MD.Enum.XS
                                    enabled: {
                                        if (!root.selected)
                                            return false;
                                        if (root.selectedKind === "canvas") {
                                            const ovr = root.selectedCanvasObject?.layoutOverride || ({});
                                            return !canvasLayoutSetQuery.querying && (ovr.fillmodeSet === true || ovr.locationSet === true || ovr.rotationSet === true);
                                        }
                                        const ovr = root.selected.layoutOverride || ({});
                                        return ovr.fillmodeSet === true || ovr.locationSet === true || ovr.alignSet === true || ovr.rotationSet === true;
                                    }
                                    icon.name: MD.Token.icon.refresh
                                    MD.ToolTip.visible: hovered
                                    MD.ToolTip.text: qsTr("Revert to global default")
                                    onClicked: root.resetLayout()
                                }
                            }
                        }

                        Flow {
                            id: layoutFlow
                            readonly property var displayLayout: {
                                if (!root.selected)
                                    return ({});
                                if (root.selectedKind === "canvas")
                                    return root.selectedCanvasObject?.effectiveLayout || ({});
                                return root.selected.displayLayout || root.selected.effectiveLayout || ({});
                            }
                            readonly property int currentX: root.clampPercent(displayLayout.locationX ?? 50)
                            readonly property int currentY: root.clampPercent(displayLayout.locationY ?? 50)
                            readonly property bool locationEnabled: !!root.selected && (displayLayout.fillmode || 0) !== 1
                            Layout.fillWidth: true
                            visible: !!root.selected && (root.selectedKind === "canvas" || !(root.selected.canvasId || "").length)
                            spacing: 12

                            ColumnLayout {
                                width: Math.min(layoutFlow.width, 220)
                                spacing: 4

                                MD.Text {
                                    text: qsTr("Fill mode")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }

                                MD.ComboBox {
                                    id: fillmodeBox
                                    Layout.fillWidth: true
                                    mdState.size: MD.Enum.S
                                    model: root.kFillModeLabels
                                    currentIndex: {
                                        if (!root.selected)
                                            return 0;
                                        return root.fillmodeIndex(layoutFlow.displayLayout.fillmode || 0);
                                    }
                                    onActivated: idx => root.applyFillmode(root.kFillModeValues[idx])
                                }
                            }

                            ColumnLayout {
                                width: Math.min(layoutFlow.width, 260)
                                spacing: 4

                                enabled: layoutFlow.locationEnabled
                                opacity: enabled ? 1.0 : 0.4

                                MD.Text {
                                    text: qsTr("Horizontal")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }

                                W.ValueSlider {
                                    id: horizontalLocation
                                    Layout.fillWidth: true
                                    from: 0
                                    to: 100
                                    stepSize: 1
                                    value: layoutFlow.currentX
                                    valueText: root.clampPercent(value)
                                    valueMaxText: root.clampPercent(to).toString()
                                    valueHorizontalAlignment: Text.AlignLeft
                                    onMoved: root.applyLocation(value, verticalLocation.value)
                                }
                            }

                            ColumnLayout {
                                width: Math.min(layoutFlow.width, 260)
                                spacing: 4

                                enabled: layoutFlow.locationEnabled
                                opacity: enabled ? 1.0 : 0.4

                                MD.Text {
                                    text: qsTr("Vertical")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }

                                W.ValueSlider {
                                    id: verticalLocation
                                    Layout.fillWidth: true
                                    from: 0
                                    to: 100
                                    stepSize: 1
                                    value: layoutFlow.currentY
                                    valueText: root.clampPercent(value)
                                    valueMaxText: root.clampPercent(to).toString()
                                    valueHorizontalAlignment: Text.AlignLeft
                                    onMoved: root.applyLocation(horizontalLocation.value, value)
                                }
                            }

                            ColumnLayout {
                                width: Math.min(layoutFlow.width, implicitWidth)
                                spacing: 4

                                MD.Text {
                                    text: qsTr("Rotation")
                                    typescale: MD.Token.typescale.label_medium
                                    color: MD.Token.color.on_surface_variant
                                }

                                MD.SegmentedButtonGroup {
                                    id: rotationGroup
                                    size: MD.Enum.XS

                                    // Inline buttons; SegmentedButtonGroup's
                                    // updatePositions only recognises segmented
                                    // buttons that are direct children — a
                                    // Repeater here ends up in contentModel as
                                    // an extra slot and shifts PosFirst off the
                                    // real first button.
                                    function applyRotation(rotationValue) {
                                        root.applyRotation(rotationValue);
                                    }
                                    function isChecked(rotationValue) {
                                        if (!root.selected)
                                            return rotationValue === 1; // ROTATION_NORMAL
                                        return (layoutFlow.displayLayout.rotation || 0) === rotationValue;
                                    }

                                    MD.SegmentedButton {
                                        text: root.kRotationLabels[0]
                                        checked: rotationGroup.isChecked(root.kRotationValues[0])
                                        onClicked: rotationGroup.applyRotation(root.kRotationValues[0])
                                    }
                                    MD.SegmentedButton {
                                        text: root.kRotationLabels[1]
                                        checked: rotationGroup.isChecked(root.kRotationValues[1])
                                        onClicked: rotationGroup.applyRotation(root.kRotationValues[1])
                                    }
                                    MD.SegmentedButton {
                                        text: root.kRotationLabels[2]
                                        checked: rotationGroup.isChecked(root.kRotationValues[2])
                                        onClicked: rotationGroup.applyRotation(root.kRotationValues[2])
                                    }
                                    MD.SegmentedButton {
                                        text: root.kRotationLabels[3]
                                        checked: rotationGroup.isChecked(root.kRotationValues[3])
                                        onClicked: rotationGroup.applyRotation(root.kRotationValues[3])
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

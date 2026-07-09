pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import Qcm.Material as MD
import waywallen.ui as W

MD.Page {
    id: root
    showBackground: false
    padding: MD.MProp.size.isCompact ? 0 : 12

    property var detailRow: null
    property int detailState: 0

    property string sourceId: ""
    property var sortOptions: []
    property int sortIndex: 0
    property var discoverTweakSheet: null

    W.TweakState {
        id: discoverTweakState
        settingsCategory: "DiscoverView"
    }

    function sourceInfo(id) {
        for (const s of availabilityQuery.sources) {
            if (s.id === id)
                return s;
        }
        return null;
    }

    function sourceName(id) {
        const s = sourceInfo(id);
        return s ? s.name : "";
    }

    function sourceTags(id) {
        const s = sourceInfo(id);
        return s && s.tags ? s.tags : [];
    }

    function sameList(a, b) {
        const left = a ?? [];
        const right = b ?? [];
        if (left.length !== right.length)
            return false;
        for (let i = 0; i < left.length; ++i) {
            if (left[i] !== right[i])
                return false;
        }
        return true;
    }

    function pruneTags(tags, allowedTags) {
        const allowed = {};
        for (const tag of allowedTags ?? [])
            allowed[String(tag)] = true;
        let out = [];
        for (const tag of tags ?? []) {
            const value = String(tag);
            if (allowed[value] === true)
                out.push(value);
        }
        return out;
    }

    function sortLabel() {
        if (sortOptions.length === 0)
            return qsTr("Sort");
        return sortOptions[Math.max(0, Math.min(sortIndex, sortOptions.length - 1))].label;
    }

    function setSource(id) {
        const s = sourceInfo(id);
        if (!s)
            return;
        const sourceChanged = sourceId !== id;
        const currentSortKey = searchQuery.sortKey;
        sourceId = id;
        sortOptions = s.sorts ?? [];
        sortIndex = 0;
        if (!sourceChanged && currentSortKey.length > 0) {
            for (let i = 0; i < sortOptions.length; ++i) {
                if (sortOptions[i].key === currentSortKey) {
                    sortIndex = i;
                    break;
                }
            }
        }
        const nextTags = sourceChanged ? [] : pruneTags(searchQuery.tags, s.tags ?? []);
        if (!sameList(searchQuery.tags, nextTags))
            searchQuery.tags = nextTags;
        searchQuery.sourceId = id;
        detailsQuery.sourceId = id;
        searchQuery.sortKey = sortOptions.length > 0 ? sortOptions[sortIndex].key : "";
        if (sourceChanged) {
            detailRow = null;
            detailState = 0;
            detailsQuery.itemId = "";
        }
    }

    function pickSort(idx) {
        if (idx < 0 || idx >= sortOptions.length)
            return;
        sortIndex = idx;
        searchQuery.sortKey = sortOptions[idx].key;
    }

    function selectItem(index) {
        detailRow = searchQuery.model.get(index);
        detailState = detailRow.installed ? 3 : 0;
        detailsQuery.sourceId = detailRow.sourceId;
        detailsQuery.itemId = detailRow.itemId;
    }

    function closeDetail() {
        detailRow = null;
        detailState = 0;
        detailsQuery.itemId = "";
        m_grid.currentIndex = -1;
    }

    function openInfo() {
        if (!root.detailRow)
            return;
        MD.Util.showPopup('waywallen.ui/PagePopup', {
            source: 'waywallen.ui/RemoteInfoPage',
            props: {
                item: root.detailRow,
                details: detailsQuery,
                sourceName: root.sourceName(root.detailRow.sourceId)
            }
        }, root.Window.window);
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
        return num.toFixed(unit === "B" ? 0 : 1) + " " + unit;
    }

    function isSheetActive(sheet) {
        return !!sheet && (sheet.opened || sheet.entering);
    }

    function ensureDiscoverTweakSheet() {
        if (root.discoverTweakSheet)
            return root.discoverTweakSheet;

        const sheet = MD.Util.showPopup(discoverTweakSheetComponent, {}, root.Window.window);
        if (sheet)
            root.discoverTweakSheet = sheet;
        return sheet;
    }

    function releaseDiscoverTweakSheet(sheet) {
        if (root.discoverTweakSheet === sheet)
            root.discoverTweakSheet = null;
    }

    function toggleDiscoverTweakSheet() {
        if (root.isSheetActive(root.discoverTweakSheet)) {
            root.discoverTweakSheet.close();
            return;
        }
        const sheet = root.ensureDiscoverTweakSheet();
        if (sheet && !sheet.opened && !sheet.entering)
            sheet.open();
    }

    MD.Action {
        id: tweakAction
        text: "Tweak"
        icon.name: MD.Token.icon.tune
        checked: root.isSheetActive(root.discoverTweakSheet)
        onTriggered: root.toggleDiscoverTweakSheet()
    }

    MD.Action {
        id: filterAction
        icon.name: MD.Token.icon.filter_list
        text: "Filters"
        enabled: m_filter_dialog.availableTags.length > 0
        checked: searchQuery.tags.length > 0
        onTriggered: m_filter_dialog.open()
    }

    MD.Action {
        id: refreshAction
        icon.name: MD.Token.icon.refresh
        text: "Refresh"
        enabled: !searchQuery.querying
        onTriggered: searchQuery.reload()
    }

    MD.Action {
        id: closeDetailAction
        text: "Close"
        icon.name: MD.Token.icon.close
        onTriggered: root.closeDetail()
    }

    MD.Action {
        id: infoAction
        text: "Info"
        icon.name: MD.Token.icon.info
        enabled: root.detailRow !== null
        onTriggered: root.openInfo()
    }

    W.RemoteAvailabilityQuery {
        id: availabilityQuery
        onSourcesChanged: {
            if (sources.length === 0)
                return;
            if (root.sourceId.length === 0)
                root.setSource(defaultSourceId.length > 0 ? defaultSourceId : sources[0].id);
            else
                root.setSource(root.sourceId);
        }
    }

    W.RemoteSearchQuery {
        id: searchQuery
        onStateChanged: {
            if (errorText.length > 0)
                W.Action.toast(qsTr("Remote search failed: ") + errorText);
        }
    }

    W.RemoteFilterDialog {
        id: m_filter_dialog
        parent: T.Overlay.overlay
        anchors.centerIn: parent
        availableTags: root.sourceTags(root.sourceId)
        selectedTags: searchQuery.tags
        onApply: function(tags) {
            searchQuery.tags = tags;
        }
    }

    W.RemoteDetailsQuery {
        id: detailsQuery
    }

    W.RemoteDownloadQuery {
        id: dlQuery
        function onUninstalled(sourceId, id) {
            searchQuery.model.setInstalled(sourceId, id, false);
            if (root.detailRow && root.detailRow.sourceId === sourceId && root.detailRow.itemId === id) {
                root.detailRow.installed = false;
                root.detailState = 0;
            }
            W.Action.toast(qsTr("Uninstalled"));
        }
        function onUninstallFailed(sourceId, id, error) {
            W.Action.toast(qsTr("Uninstall failed: ") + error);
        }
        function onRejected(sourceId, id, error) {
            if (root.detailRow && root.detailRow.sourceId === sourceId && root.detailRow.itemId === id)
                root.detailState = 0;
            W.Action.toast(qsTr("Download rejected: ") + error);
        }
    }

    Connections {
        target: W.Notify
        function onRemoteDownloadProgress(sourceId, id, state, error) {
            if (state === 3)
                searchQuery.model.setInstalled(sourceId, id, true);
            if (root.detailRow && root.detailRow.sourceId === sourceId && root.detailRow.itemId === id) {
                root.detailState = state;
                if (state === 3)
                    root.detailRow.installed = true;
            }
            if (state === 5 && error.length > 0)
                W.Action.toast(qsTr("Download failed: ") + error);
        }
    }

    function reloadAll() {
        availabilityQuery.reload();
        if (root.sourceId.length > 0)
            searchQuery.reload();
    }

    Connections {
        target: W.Notify
        function onDaemonReady() {
            root.reloadAll();
        }
    }

    Component.onCompleted: {
        if (W.Notify.daemonPhase === W.Notify.DaemonPhase.Ready)
            reloadAll();
    }

    contentItem: RowLayout {
        spacing: 12

        MD.Pane {
            Layout.fillWidth: true
            Layout.fillHeight: true
            radius: root.MD.MProp.page.backgroundRadius
            padding: 0
            showBackground: true

            contentItem: ColumnLayout {
                spacing: 0

                RowLayout {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    Layout.topMargin: 4
                    spacing: 8

                    MD.EmbedChip {
                        id: sourceChip
                        visible: availabilityQuery.sources.length > 1
                        text: root.sourceName(root.sourceId)
                        trailingIconName: MD.Token.icon.arrow_drop_down
                        mdState.borderWidth: 1
                        onClicked: sourceMenu.open()

                        MD.Menu {
                            id: sourceMenu
                            parent: sourceChip
                            y: parent.height
                            model: availabilityQuery.sources
                            contentDelegate: MD.MenuItem {
                                required property var modelData
                                text: modelData.name
                                icon.name: modelData.id === root.sourceId ? MD.Token.icon.check : ' '
                                onClicked: {
                                    root.setSource(modelData.id);
                                    sourceMenu.close();
                                }
                            }
                        }
                    }

                    MD.EmbedChip {
                        id: sortChip
                        visible: root.sortOptions.length > 0
                        text: root.sortLabel()
                        trailingIconName: MD.Token.icon.arrow_drop_down
                        mdState.borderWidth: 1
                        onClicked: sortMenu.open()

                        MD.Menu {
                            id: sortMenu
                            parent: sortChip
                            y: parent.height
                            model: root.sortOptions
                            contentDelegate: MD.MenuItem {
                                required property var modelData
                                required property int index
                                text: modelData.label
                                icon.name: index === root.sortIndex ? MD.Token.icon.check : ' '
                                onClicked: {
                                    root.pickSort(index);
                                    sortMenu.close();
                                }
                            }
                        }
                    }

                    W.SearchChip {
                        id: m_search_field
                        Layout.preferredWidth: 120
                        placeholderText: qsTr("Search")
                        onTextEdited: searchQuery.query = text
                    }

                    MD.ActionToolBar {
                        Layout.fillWidth: true
                        actions: [tweakAction, filterAction, refreshAction]
                    }
                }

                MD.LinearIndicator {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    visible: searchQuery.querying && searchQuery.model.count > 0
                    running: visible
                }

                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    MD.VerticalGridView {
                        id: m_grid
                        anchors.fill: parent
                        clip: true
                        cacheBuffer: 300
                        displayMarginBeginning: 300
                        displayMarginEnd: 300
                        currentIndex: -1
                        topMargin: 2
                        bottomMargin: 8
                        leftMargin: 8
                        rightMargin: 8

                        readonly property real _availableWidth: Math.max(0, width - leftMargin - rightMargin)
                        readonly property int _cols: Math.max(1, Math.floor(_availableWidth / discoverTweakState.itemSize))
                        readonly property real _stretchedItemWidth: _availableWidth / _cols
                        readonly property bool _fillCell: discoverTweakState.layoutMode === discoverTweakState.layoutFillCell
                        readonly property real _displayItemWidth: _fillCell ? _stretchedItemWidth : Math.min(discoverTweakState.itemSize, _stretchedItemWidth)
                        readonly property real _displayItemHeight: _displayItemWidth / Math.max(discoverTweakState.itemAspectRatio, 0.1)
                        cellWidth: _stretchedItemWidth
                        cellHeight: _fillCell ? _displayItemHeight : discoverTweakState.itemHeight

                        model: searchQuery.model

                        delegate: RemoteCard {
                            itemWidth: m_grid._displayItemWidth
                            itemHeight: m_grid._displayItemHeight
                            onClicked: {
                                m_grid.currentIndex = index;
                                root.selectItem(index);
                            }
                        }

                        highlightFollowsCurrentItem: true
                        highlight: Component {
                            Item {
                                visible: m_grid.currentItem !== null
                                z: 2
                                Rectangle {
                                    anchors.fill: parent
                                    anchors.margins: 4
                                    color: "transparent"
                                    border.color: MD.Token.color.primary
                                    border.width: 3
                                    radius: MD.Token.shape.corner.small + 2
                                }
                            }
                        }
                    }

                    ColumnLayout {
                        anchors.centerIn: parent
                        visible: m_grid.count === 0
                        spacing: 8

                        MD.BusyIndicator {
                            Layout.alignment: Qt.AlignHCenter
                            running: searchQuery.querying
                            visible: running
                        }

                        MD.Label {
                            Layout.alignment: Qt.AlignHCenter
                            visible: !searchQuery.querying
                            text: qsTr("No wallpapers found")
                            typescale: MD.Token.typescale.body_large
                            color: MD.Token.color.on_surface_variant
                        }
                    }
                }
            }
        }

        MD.Pane {
            Layout.preferredWidth: 300
            Layout.maximumWidth: 300
            Layout.fillHeight: true
            visible: root.detailRow !== null
            radius: root.MD.MProp.page.backgroundRadius
            padding: 0
            showBackground: true

            contentItem: ColumnLayout {
                spacing: 12

                Rectangle {
                    Layout.fillWidth: true
                    Layout.topMargin: 16
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    Layout.preferredHeight: width * 0.56
                    radius: MD.Token.shape.corner.medium
                    clip: true
                    color: MD.Token.color.surface_container

                    AnimatedImage {
                        anchors.fill: parent
                        source: root.detailRow ? root.detailRow.previewUrl : ""
                        fillMode: Image.PreserveAspectCrop
                        horizontalAlignment: Image.AlignHCenter
                        verticalAlignment: Image.AlignVCenter
                        smooth: true
                        cache: true
                        playing: true
                    }
                }

                Flickable {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    clip: true
                    contentWidth: width
                    contentHeight: m_info.implicitHeight
                    boundsBehavior: Flickable.StopAtBounds

                    ColumnLayout {
                        id: m_info
                        width: parent.width
                        spacing: 8

                        MD.Label {
                            Layout.fillWidth: true
                            text: root.detailRow ? root.detailRow.title : ""
                            typescale: MD.Token.typescale.title_medium
                            wrapMode: Text.WordWrap
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8

                            MD.Label {
                                Layout.fillWidth: true
                                text: root.detailRow ? root.detailRow.wpType : ""
                                typescale: MD.Token.typescale.label_large
                                color: MD.Token.color.on_surface_variant
                                maximumLineCount: 1
                                elide: Text.ElideRight
                            }

                            MD.ActionToolBar {
                                actions: [infoAction, closeDetailAction]
                                iconDelegate: MD.SmallIconButton {
                                    action: MD.ToolBarLayout.action
                                }
                            }
                        }

                        MD.Label {
                            Layout.fillWidth: true
                            text: root.detailRow ? qsTr("by ") + root.detailRow.author : ""
                            visible: root.detailRow && root.detailRow.author.length > 0
                            typescale: MD.Token.typescale.body_medium
                            color: MD.Token.color.on_surface_variant
                            wrapMode: Text.WordWrap
                        }

                        GridLayout {
                            id: m_meta
                            Layout.fillWidth: true
                            Layout.topMargin: 4
                            columns: 2
                            columnSpacing: 12
                            rowSpacing: 4
                            visible: hasResolution || hasSize

                            readonly property bool hasResolution: detailsQuery.width > 0 && detailsQuery.height > 0
                            readonly property string formattedSize: root.formatSize(detailsQuery.size)
                            readonly property bool hasSize: formattedSize.length > 0

                            MD.Text {
                                visible: m_meta.hasResolution
                                text: "Resolution"
                                typescale: MD.Token.typescale.label_medium
                                color: MD.Token.color.on_surface_variant
                            }
                            MD.Text {
                                visible: m_meta.hasResolution
                                text: detailsQuery.width + "×" + detailsQuery.height
                                typescale: MD.Token.typescale.body_medium
                                color: MD.Token.color.on_surface
                            }

                            MD.Text {
                                visible: m_meta.hasSize
                                text: "Size"
                                typescale: MD.Token.typescale.label_medium
                                color: MD.Token.color.on_surface_variant
                            }
                            MD.Text {
                                visible: m_meta.hasSize
                                text: m_meta.formattedSize
                                typescale: MD.Token.typescale.body_medium
                                color: MD.Token.color.on_surface
                            }
                        }

                        Flow {
                            Layout.fillWidth: true
                            Layout.topMargin: 4
                            spacing: 4
                            visible: detailsQuery.tags.length > 0

                            Repeater {
                                model: detailsQuery.tags
                                delegate: W.Tag {
                                    required property string modelData
                                    text: modelData
                                }
                            }
                        }

                        MD.Divider {
                            Layout.fillWidth: true
                            Layout.topMargin: 4
                            visible: detailsQuery.description.length > 0 || detailsQuery.querying
                        }

                        MD.Text {
                            visible: detailsQuery.description.length > 0 || detailsQuery.querying
                            text: "Description"
                            typescale: MD.Token.typescale.label_large
                            color: MD.Token.color.on_surface_variant
                        }
                        MD.Label {
                            Layout.fillWidth: true
                            text: detailsQuery.querying ? qsTr("Loading…") : detailsQuery.description
                            visible: text.length > 0
                            typescale: MD.Token.typescale.body_medium
                            color: MD.Token.color.on_surface
                            wrapMode: Text.WordWrap
                        }
                    }
                }

                MD.Button {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    Layout.bottomMargin: 16
                    mdState.type: root.detailState === 3 ? MD.Enum.BtFilledTonal : MD.Enum.BtFilled
                    enabled: root.detailState === 0 || root.detailState === 3
                    text: {
                        switch (root.detailState) {
                        case 1: return qsTr("Pending");
                        case 2: return qsTr("Downloading");
                        case 3: return qsTr("Uninstall");
                        case 4: return qsTr("Retry");
                        case 5: return qsTr("Retry");
                        default: return qsTr("Download");
                        }
                    }
                    onClicked: {
                        if (!root.detailRow) return;
                        if (root.detailState === 3) {
                            dlQuery.uninstall(root.detailRow.sourceId, root.detailRow.itemId);
                        } else {
                            root.detailState = 1;
                            dlQuery.start(root.detailRow.sourceId, root.detailRow.itemId);
                        }
                    }
                }
            }
        }
    }

    Component {
        id: discoverTweakSheetComponent

        W.TweakSheet {
            popupParent: root
            tweak: discoverTweakState
            onReleased: function (sheet) {
                root.releaseDiscoverTweakSheet(sheet);
            }
        }
    }
}

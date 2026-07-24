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

    property bool detailOpen: false
    readonly property var detailRow: detailOpen ? detailStore.item : null

    property string sourceId: ""
    property var sortOptions: []
    property int sortIndex: 0
    property var discoverTweakSheet: null
    readonly property var discoverTweakState: discoverTweakStateLoader.item

    Loader {
        id: discoverTweakStateLoader
        active: true
        property string settingsCategory: "DiscoverView"

        sourceComponent: Component {
            W.TweakState {
                settingsCategory: discoverTweakStateLoader.settingsCategory
            }
        }
    }

    W.DiscoverState {
        id: discoverState
    }

    W.RemoteStoreItem {
        id: detailStore
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
        return s ? (s.displayName && s.displayName.length > 0 ? s.displayName : s.name) : "";
    }

    function sourceFilters(id) {
        const s = sourceInfo(id);
        return s && s.filters ? s.filters : [];
    }

    function sourceCapability(id) {
        const s = sourceInfo(id);
        return s ? Number(s.remoteCapability ?? 0) : 0;
    }

    function sourceRemoteHint(id) {
        const s = sourceInfo(id);
        return s ? String(s.remoteHint ?? "") : "";
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

    function pruneTags(tags, filters) {
        const allowed = {};
        for (const filter of filters ?? []) {
            for (const value of filter.values ?? [])
                allowed[String(value)] = true;
        }
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

    function tweakSettingsCategory(id) {
        const source = String(id ?? "");
        return source.length > 0
            ? "DiscoverView/" + encodeURIComponent(source) + "/Tweak"
            : "DiscoverView";
    }

    function loadDiscoverTweakState(id) {
        const category = tweakSettingsCategory(id);
        if (discoverTweakStateLoader.item
                && discoverTweakStateLoader.settingsCategory === category)
            return;
        if (isSheetActive(discoverTweakSheet))
            discoverTweakSheet.close();
        discoverTweakStateLoader.active = false;
        discoverTweakStateLoader.settingsCategory = category;
        discoverTweakStateLoader.active = true;
    }

    function setSource(id) {
        const s = sourceInfo(id);
        if (!s)
            return;
        const sourceChanged = sourceId !== id;
        const currentSortKey = searchQuery.sortKey;
        sourceId = id;
        discoverState.setSelectedSourceId(id);
        if (sourceChanged)
            loadDiscoverTweakState(id);
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
        const canBrowse = currentSourceLoginAction() === null;
        searchQuery.browsingEnabled = canBrowse;
        const candidateTags = sourceChanged
            ? discoverState.filterValuesFor(id)
            : searchQuery.tags;
        const nextTags = pruneTags(candidateTags, s.filters ?? []);
        if (!sameList(searchQuery.tags, nextTags))
            searchQuery.tags = nextTags;
        discoverState.setFilterValuesFor(id, nextTags);
        searchQuery.sourceId = id;
        detailsQuery.sourceId = id;
        searchQuery.sortKey = sortOptions.length > 0 ? sortOptions[sortIndex].key : "";
        if (sourceChanged || !canBrowse) {
            detailOpen = false;
            detailsQuery.itemId = "";
            m_grid.currentIndex = -1;
        }
    }

    function pickSort(idx) {
        if (idx < 0 || idx >= sortOptions.length)
            return;
        sortIndex = idx;
        searchQuery.sortKey = sortOptions[idx].key;
    }

    function selectItem(index) {
        detailStore.item = searchQuery.model.item(index);
        detailOpen = true;
        detailsQuery.sourceId = detailRow.sourceId;
        detailsQuery.itemId = detailRow.itemId;
        if (root.sourceCapability(detailRow.sourceId) === 2)
            refreshDetailSubscription();
    }

    function closeDetail() {
        detailOpen = false;
        detailsQuery.itemId = "";
        m_grid.currentIndex = -1;
    }

    function openInfo() {
        if (!root.detailRow)
            return;
        MD.Util.showPopup('waywallen.ui/PagePopup', {
            source: 'waywallen.ui/RemoteInfoPage',
            props: {
                itemStore: detailStore,
                details: detailsQuery,
                sourceName: root.sourceName(root.detailRow.sourceId),
                remoteCapability: root.sourceCapability(root.detailRow.sourceId),
                remoteHint: root.sourceRemoteHint(root.detailRow.sourceId)
            }
        }, root.Window.window);
    }

    function setDetailSubscription(subscribed) {
        if (!root.detailRow)
            return;
        const sourceId = root.detailRow.sourceId;
        const itemId = root.detailRow.itemId;
        subscriptionQuery.setSubscribed(sourceId, itemId, subscribed);
    }

    function refreshDetailSubscription() {
        if (!root.detailRow || root.sourceCapability(root.detailRow.sourceId) !== 2)
            return;
        if (root.currentSourceLoginAction() !== null)
            return;
        subscriptionQuery.refresh(root.detailRow.sourceId, root.detailRow.itemId);
    }

    function isSheetActive(sheet) {
        return !!sheet && (sheet.opened || sheet.entering);
    }

    function ensureDiscoverTweakSheet() {
        if (!root.discoverTweakState)
            return null;
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
        id: manageAction
        icon.name: MD.Token.icon.manage_accounts
        text: qsTr("Manage")
        enabled: root.currentSourceConfig() !== null
        onTriggered: root.openRemoteManagement()
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
        enabled: m_filter_dialog.filters.length > 0
        checked: searchQuery.tags.length > 0
        onTriggered: m_filter_dialog.open()
    }

    MD.Action {
        id: refreshAction
        icon.name: MD.Token.icon.refresh
        text: "Refresh"
        enabled: searchQuery.browsingEnabled && !searchQuery.querying
        onTriggered: searchQuery.reload()
    }

    W.RemoteAvailabilityQuery {
        id: availabilityQuery
        onSourcesChanged: {
            if (sources.length === 0) {
                searchQuery.browsingEnabled = false;
                return;
            }
            let nextSourceId = root.sourceId;
            if (!root.sourceInfo(nextSourceId)) {
                const savedSourceId = discoverState.selectedSourceId();
                if (root.sourceInfo(savedSourceId))
                    nextSourceId = savedSourceId;
                else if (root.sourceInfo(defaultSourceId))
                    nextSourceId = defaultSourceId;
                else
                    nextSourceId = sources[0].id;
            }
            root.setSource(nextSourceId);
            root.refreshDetailSubscription();
        }
    }

    function currentSourceConfig() {
        const id = root.sourceId;
        const list = availabilityQuery.sources || [];
        for (let i = 0; i < list.length; ++i) {
            if (list[i].id === id)
                return list[i];
        }
        return null;
    }

    function currentSourceLoginAction() {
        const c = root.currentSourceConfig();
        const actions = c && c.actions ? c.actions : [];
        for (let i = 0; i < actions.length; ++i) {
            const kind = Number(actions[i].kind);
            if ((kind === 2 || kind === 3) && actions[i].requiredForBrowsing === true
                && (actions[i].visible === undefined || actions[i].visible))
                return actions[i];
        }
        return null;
    }

    function openRemoteManagement() {
        const c = root.currentSourceConfig();
        if (!c)
            return;
        MD.Util.showPopup('waywallen.ui/PagePopup', {
            source: 'waywallen.ui/RemoteManagePage',
            props: {
                sourceId: c.id,
                displayName: (c.displayName && c.displayName.length > 0) ? c.displayName : (c.name || c.id)
            }
        }, root.Window.window);
    }

    W.RemoteSearchQuery {
        id: searchQuery
    }

    W.RemoteFilterDialog {
        id: m_filter_dialog
        parent: T.Overlay.overlay
        anchors.centerIn: parent
        filters: root.sourceFilters(root.sourceId)
        selectedValues: searchQuery.tags
        onApply: function (values) {
            searchQuery.tags = values;
            discoverState.setFilterValuesFor(root.sourceId, values);
        }
    }

    W.RemoteDetailsQuery {
        id: detailsQuery
    }

    W.RemoteDownloadQuery {
        id: dlQuery
        function onRemoved(sourceId, id) {
            W.Action.toast(qsTr("Removed"));
        }
        function onRemoveFailed(sourceId, id, error) {
            W.Action.toast(qsTr("Remove failed: ") + error);
        }
        function onRejected(sourceId, id, error) {
            W.Action.toast(qsTr("Download rejected: ") + error);
        }
    }

    W.RemoteSubscriptionQuery {
        id: subscriptionQuery
        forwardError: false
    }

    Connections {
        target: subscriptionQuery
        function onStateLoaded(sourceId, id, state, error) {
            if (error.length > 0)
                W.Action.toast(qsTr("Couldn't load subscription status: ") + error);
        }
        function onSetFinished(sourceId, id, subscribed, accepted, error) {
            if (accepted) {
                const message = subscribed ? qsTr("Subscription request accepted") : qsTr("Unsubscribe request accepted");
                const hint = root.sourceRemoteHint(sourceId);
                W.Action.toast(hint.length > 0 ? message + "\n" + hint : message);
            } else {
                W.Action.toast(qsTr("Subscription update failed: ") + error);
            }
        }
    }

    Connections {
        target: W.Notify
        function onRemoteDownloadProgress(sourceId, id, state, error) {
            if (state === 5 && error.length > 0)
                W.Action.toast(qsTr("Download failed: ") + error);
        }
    }

    function reloadAll() {
        availabilityQuery.reload();
    }

    Connections {
        target: W.Notify
        function onDaemonReady() {
            root.reloadAll();
        }
        function onSettingsChanged() {
            availabilityQuery.reload();
        }
        function onPluginStateChanged() {
            availabilityQuery.reload();
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
                                text: modelData.displayName && modelData.displayName.length > 0 ? modelData.displayName : modelData.name
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
                        actions: [manageAction, tweakAction, filterAction, refreshAction]
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
                        readonly property int _cols: Math.max(1, Math.floor(_availableWidth / root.discoverTweakState.itemSize))
                        readonly property real _stretchedItemWidth: _availableWidth / _cols
                        readonly property bool _fillCell: root.discoverTweakState.layoutMode === root.discoverTweakState.layoutFillCell
                        readonly property real _displayItemWidth: _fillCell ? _stretchedItemWidth : Math.min(root.discoverTweakState.itemSize, _stretchedItemWidth)
                        readonly property real _displayItemHeight: _displayItemWidth / Math.max(root.discoverTweakState.itemAspectRatio, 0.1)
                        cellWidth: _stretchedItemWidth
                        cellHeight: _fillCell ? _displayItemHeight : root.discoverTweakState.itemHeight

                        model: searchQuery.model

                        delegate: RemoteCard {
                            remoteCapability: root.sourceCapability(root.sourceId)
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
                        width: Math.min(parent.width - 48, 420)
                        visible: m_grid.count === 0
                        spacing: 12

                        readonly property bool hasError: !searchQuery.querying && searchQuery.errorText.length > 0
                        readonly property var loginAction: root.currentSourceLoginAction()
                        readonly property bool needsLogin: loginAction !== null
                        readonly property string loginLabel: {
                            const label = loginAction ? String(loginAction.label ?? "").trim() : "";
                            return label.length > 0 ? label : qsTr("Log in to %1").arg(root.sourceName(root.sourceId));
                        }
                        readonly property string loginDescription: {
                            const browseDescription = loginAction
                                ? String(loginAction.browseDescription ?? "").trim() : "";
                            if (browseDescription.length > 0)
                                return browseDescription;
                            const description = loginAction
                                ? String(loginAction.description ?? "").trim() : "";
                            return description.length > 0
                                ? description : qsTr("Log in to start browsing.");
                        }
                        readonly property string loginButtonLabel: {
                            const label = loginAction
                                ? String(loginAction.browseButtonLabel ?? "").trim() : "";
                            return label.length > 0 ? label : qsTr("Log in");
                        }

                        MD.BusyIndicator {
                            Layout.alignment: Qt.AlignHCenter
                            running: searchQuery.querying
                            visible: running
                        }

                        MD.Icon {
                            Layout.alignment: Qt.AlignHCenter
                            visible: parent.needsLogin
                            name: MD.Token.icon.person
                            size: 40
                            color: MD.Token.color.on_surface_variant
                        }
                        MD.Label {
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                            visible: parent.needsLogin || parent.hasError
                            text: parent.needsLogin ? parent.loginLabel : qsTr("Couldn't load this source")
                            typescale: MD.Token.typescale.title_medium
                            color: MD.Token.color.on_surface
                            wrapMode: Text.WordWrap
                        }
                        MD.Label {
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                            visible: parent.needsLogin || parent.hasError
                            text: parent.needsLogin ? parent.loginDescription : searchQuery.errorText
                            typescale: MD.Token.typescale.body_medium
                            color: MD.Token.color.on_surface_variant
                            font.capitalization: Font.MixedCase
                            wrapMode: Text.WordWrap
                            maximumLineCount: 4
                            elide: Text.ElideRight
                        }
                        MD.Button {
                            Layout.alignment: Qt.AlignHCenter
                            visible: parent.needsLogin
                            text: parent.loginButtonLabel
                            mdState.type: MD.Enum.BtFilledTonal
                            onClicked: root.openRemoteManagement()
                        }

                        MD.Label {
                            Layout.alignment: Qt.AlignHCenter
                            visible: !searchQuery.querying && !parent.needsLogin
                                     && searchQuery.errorText.length === 0
                            text: qsTr("No wallpapers found")
                            typescale: MD.Token.typescale.body_large
                            color: MD.Token.color.on_surface_variant
                        }
                    }
                }
            }
        }

        MD.Pane {
            Layout.preferredWidth: root.detailRow !== null ? 280 : 0
            Layout.maximumWidth: 280
            Layout.fillHeight: true
            visible: root.detailRow !== null
            radius: root.MD.MProp.page.backgroundRadius
            padding: 0
            showBackground: true

            contentItem: RemoteDetailPanel {
                item: root.detailRow
                details: detailsQuery
                remoteCapability: root.detailRow
                    ? root.sourceCapability(root.detailRow.sourceId) : 0
                remoteHint: root.detailRow
                    ? root.sourceRemoteHint(root.detailRow.sourceId) : ""
                downloadState: Number(root.detailRow?.acquisitionState ?? 0)
                subscriptionState: Number(root.detailRow?.acquisitionState ?? 0)

                onBack: root.closeDetail()
                onShowInfo: root.openInfo()
                onDownloadRequested: {
                    if (!root.detailRow)
                        return;
                    dlQuery.start(root.detailRow.sourceId, root.detailRow.itemId);
                }
                onRemoveRequested: {
                    if (root.detailRow)
                        dlQuery.remove(root.detailRow.sourceId, root.detailRow.itemId);
                }
                onSubscriptionRefreshRequested: root.refreshDetailSubscription()
                onSubscriptionChangeRequested: subscribed => root.setDetailSubscription(subscribed)
            }
        }
    }

    Component {
        id: discoverTweakSheetComponent

        W.TweakSheet {
            popupParent: root
            tweak: root.discoverTweakState
            onReleased: function (sheet) {
                root.releaseDiscoverTweakSheet(sheet);
            }
        }
    }
}

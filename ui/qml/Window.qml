pragma ComponentBehavior: Bound
pragma ValueTypeBehavior: Assertable
import QtCore
import QtQuick
import QtQml
import QtQuick.Window
import QtQuick.Layouts
import QtQuick.Templates as T

import Qcm.Material as MD
import waywallen.ui as W

MD.ApplicationWindow {
    id: win
    MD.MProp.size.width: width
    MD.MProp.backgroundColor: {
        return MD.MProp.color.surface_container;
    }
    MD.MProp.textColor: MD.MProp.color.getOn(MD.MProp.backgroundColor)

    color: MD.MProp.backgroundColor
    visible: true
    height: 632
    width: 948
    title: "waywallen"

    readonly property alias popupPresenter: m_popup_presenter

    function presentPopup(source, properties) {
        const presentation = m_popup_presenter.present(source, properties || {});
        presentation.failed.connect(presentation, function (error) {
            W.Global.toastError(error);
        });
        if (presentation.status === MD.PopupPresentation.Error)
            W.Global.toastError(presentation.errorString);
        return presentation;
    }

    MD.PopupPresenter {
        id: m_popup_presenter
        host: win.contentItem
    }

    // Persist the window size across runs. Wayland doesn't let clients
    // restore their own position, so only width/height are stored.
    Settings {
        category: "window"
        property alias width: win.width
        property alias height: win.height
    }

    W.HealthQuery {
        id: healthQuery
    }

    W.GlobalPauseToggleQuery {
        id: globalPauseToggleQuery
        onToggled: paused => W.Action.toast(paused ? qsTr("Paused") : qsTr("Resumed"))
    }

    Shortcut {
        sequence: "Ctrl+P"
        context: Qt.ApplicationShortcut
        enabled: W.Notify.daemonPhase === W.Notify.DaemonPhase.Ready && !globalPauseToggleQuery.querying
        onActivated: globalPauseToggleQuery.reload()
    }

    Connections {
        target: W.Notify
        function onDaemonReady() {
            healthQuery.reload();
        }
    }

    property int currentPage: 0

    readonly property bool isCompact: MD.MProp.size.isCompact

    readonly property var pageModel: [
        {
            icon: MD.Token.icon.wallpaper,
            name: qsTr("Wallpapers")
        },
        {
            icon: MD.Token.icon.explore,
            name: qsTr("Discover")
        },
        {
            icon: MD.Token.icon.monitor,
            name: qsTr("Displays")
        },
        {
            icon: MD.Token.icon.monitor_heart,
            name: qsTr("Status")
        }
    ]

    readonly property var pageComponents: ["qrc:/waywallen/ui/qml/page/WallpaperPage.qml", "qrc:/waywallen/ui/qml/page/DiscoverPage.qml", "qrc:/waywallen/ui/qml/page/DisplaysPage.qml", "qrc:/waywallen/ui/qml/page/StatusPage.qml"]

    readonly property var pageCacheable: [true, true, false, false]

    onCurrentPageChanged: {
        m_content.switchTo(pageComponents[currentPage], {}, pageCacheable[currentPage]);
    }

    Component.onCompleted: {
        MD.Token.color.useSysAccentColor = true;
        currentPageChanged();
        // Level-check for the case where the daemon is already Ready
        // before this window finishes constructing (UI launched
        // standalone against a running daemon, page reload, etc.)
        // — `daemonReady` is edge-triggered and won't fire then.
        if (W.Notify.daemonPhase === W.Notify.DaemonPhase.Ready) {
            healthQuery.reload();
        }
    }

    MD.SnakeView {
        id: m_snake
        parent: T.Overlay.overlay
        anchors.fill: parent
    }

    Connections {
        target: W.Action
        function onToast(text, duration, flags, action) {
            m_snake.show(text, duration, flags, action);
        }
    }

    Connections {
        target: W.App
        function onErrorOccurred(error) {
            W.Global.toastError(error);
        }
    }

    // Global daemon-event toasts. Notify mirrors `GlobalEvent` from the
    // daemon; library additions surface here so the toast fires no
    // matter which page triggered the add (manual vs auto-detect).
    Connections {
        target: W.Notify
        function onLibrariesAdded(paths) {
            const n = paths.length;
            W.Action.toast(qsTr("%n library(s) added", "", n));
        }
        function onDisplayConnectionFailed(clientName, clientProtocolVersion, errorCode, reason) {
            const who = clientName.length > 0 ? clientName : qsTr("Display client");
            W.Global.toastError(qsTr("%1 connection failed: %2").arg(who).arg(reason));
        }
        function onPluginRestartFailed(pluginId, error) {
            const who = pluginId.length > 0 ? pluginId : qsTr("Plugin");
            W.Action.toast(qsTr("%1 renderer restart failed: %2").arg(who).arg(error), 6000, 1, null);
        }
    }

    W.DaemonNotRunDialog {}
    W.QrLoginDialog {}

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            // --- Navigation rail (collapses to 96dp; auto-expands when
            // the window is wide enough to embed) ---
            Loader {
                id: m_drawer_loader
                Layout.fillHeight: true
                active: !win.isCompact
                visible: active

                sourceComponent: MD.NavigationRail {
                    id: m_rail
                    model: win.pageModel
                    currentIndex: win.currentPage

                    autoExpand: W.Global.sidebarAutoExpand
                    // The rail only re-syncs `expanded` on window-class
                    // changes; apply a runtime toggle of the setting at once.
                    onAutoExpandChanged: if (autoExpand)
                        expanded = useEmbed

                    onClicked: function (model) {
                        win.currentPage = model.index;
                    }

                    // Logo + a menu-toggle button (the rail's default header
                    // is just the toggle; we add branding alongside it).
                    header: Item {
                        implicitWidth: m_rail.useLarge ? m_rail.expandedWidth : m_rail.collapsedWidth
                        implicitHeight: m_logo.y + m_logo.height + 12

                        MD.StandardIconButton {
                            id: m_menu_btn
                            x: m_rail.useLarge ? (32 - (width - 24) / 2) : (m_rail.collapsedWidth - width) / 2
                            y: 4
                            icon.name: m_rail.useLarge ? MD.Token.icon.menu_open : MD.Token.icon.menu
                            onClicked: m_rail.toggle()

                            Behavior on x {
                                NumberAnimation {
                                    duration: MD.Token.duration.long2
                                    easing: MD.Token.easing.emphasized
                                }
                            }
                        }

                        Image {
                            id: m_logo
                            width: 32
                            height: 32
                            x: m_rail.useLarge ? 32 : (m_rail.collapsedWidth - width) / 2
                            y: m_menu_btn.y + m_menu_btn.height + 16
                            source: "qrc:/waywallen/ui/assets/waywallen-ui.svg"
                            fillMode: Image.PreserveAspectFit
                            sourceSize.width: 64
                            sourceSize.height: 64

                            Behavior on x {
                                NumberAnimation {
                                    duration: MD.Token.duration.long2
                                    easing: MD.Token.easing.emphasized
                                }
                            }
                        }

                        MD.Label {
                            visible: m_rail.useLarge
                            anchors.left: m_logo.right
                            anchors.leftMargin: 12
                            anchors.verticalCenter: m_logo.verticalCenter
                            text: "waywallen"
                            typescale: MD.Token.typescale.title_large
                        }
                    }

                    footer: Item {
                        implicitHeight: m_rail_footer.implicitHeight

                        Column {
                            id: m_rail_footer
                            width: parent.width
                            spacing: m_rail.useLarge ? 0 : 12

                            MD.RailItem {
                                width: parent.width
                                expand: m_rail.useLarge
                                checked: false
                                icon.name: MD.Token.icon.extension
                                iconStyle: m_rail.useLarge ? MD.Enum.IconAndText : MD.Enum.IconOnly
                                text: qsTr("Plugins")
                                property var presentation: null
                                onClicked: {
                                    if (presentation?.active)
                                        return;
                                    presentation = win.presentPopup('waywallen.ui/PagePopup', {
                                        source: 'waywallen.ui/PluginManagePage'
                                    });
                                }
                            }

                            MD.RailItem {
                                width: parent.width
                                expand: m_rail.useLarge
                                checked: false
                                icon.name: MD.Token.icon.settings
                                iconStyle: m_rail.useLarge ? MD.Enum.IconAndText : MD.Enum.IconOnly
                                text: qsTr("Settings")
                                property var presentation: null
                                onClicked: {
                                    if (presentation?.active)
                                        return;
                                    presentation = win.presentPopup('waywallen.ui/PagePopup', {
                                        source: 'waywallen.ui/SettingsPage'
                                    });
                                }
                            }

                            MD.RailItem {
                                visible: m_rail.useLarge
                                width: parent.width
                                expand: true
                                checked: false
                                icon.name: MD.Token.icon.info
                                text: qsTr("About")
                                property var presentation: null
                                onClicked: {
                                    if (presentation?.active)
                                        return;
                                    presentation = win.presentPopup('waywallen.ui/PagePopup', {
                                        source: 'waywallen.ui/AboutPage'
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // --- Page content ---
            MD.PageContainer {
                id: m_content
                Layout.fillHeight: true
                Layout.fillWidth: true
                clip: true
                initialItem: Item {}

                MD.MProp.page: m_page_ctx

                MD.PageContext {
                    id: m_page_ctx
                    showHeader: false
                    backgroundRadius: win.isCompact ? 0 : MD.Token.shape.corner.large
                    showBackground: !win.isCompact
                }
            }
        }

        // --- Bottom navigation bar (compact mode) ---
        Loader {
            id: m_bar_loader
            Layout.fillWidth: true
            active: win.isCompact
            visible: active

            sourceComponent: MD.Pane {
                padding: 0
                backgroundColor: MD.MProp.color.surface_container
                elevation: MD.Token.elevation.level2

                contentItem: RowLayout {
                    Repeater {
                        model: win.pageModel

                        Item {
                            Layout.fillWidth: true
                            implicitHeight: 12 + children[0].implicitHeight + 16
                            required property var modelData
                            required property int index

                            MD.BarItem {
                                anchors.fill: parent
                                anchors.topMargin: 12
                                anchors.bottomMargin: 16
                                icon.name: parent.modelData.icon
                                text: parent.modelData.name
                                checked: win.currentPage === parent.index
                                onClicked: win.currentPage = parent.index
                            }
                        }
                    }
                }
            }
        }
    }
}

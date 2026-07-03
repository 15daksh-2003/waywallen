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

    Connections {
        target: W.Notify
        function onDaemonReady() {
            healthQuery.reload();
        }
    }

    property int currentPage: 0

    readonly property bool isCompact: MD.MProp.size.isCompact

    readonly property var pageModel: [
        { icon: MD.Token.icon.wallpaper, name: "Wallpapers" },
        { icon: MD.Token.icon.explore, name: "Discover" },
        { icon: MD.Token.icon.monitor, name: "Displays" },
        { icon: MD.Token.icon.monitor_heart, name: "Status" }
    ]


    readonly property var pageComponents: ["qrc:/waywallen/ui/qml/page/WallpaperPage.qml", "qrc:/waywallen/ui/qml/page/DiscoverPage.qml", "qrc:/waywallen/ui/qml/page/DisplaysPage.qml", "qrc:/waywallen/ui/qml/page/StatusPage.qml"]

    readonly property var pageCacheable: [true, true, false, false]

    onCurrentPageChanged: {
        m_content.switchTo(pageComponents[currentPage], {}, pageCacheable[currentPage]);
    }

    Component.onCompleted: {
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

    // Global daemon-event toasts. Notify mirrors `GlobalEvent` from the
    // daemon; library additions surface here so the toast fires no
    // matter which page triggered the add (manual vs auto-detect).
    Connections {
        target: W.Notify
        function onLibrariesAdded(paths) {
            const n = paths.length;
            W.Action.toast(n === 1 ? "Library added" : (n + " libraries added"));
        }
        function onDisplayConnectionFailed(clientName, clientProtocolVersion, errorCode, reason) {
            const who = clientName.length > 0 ? clientName : qsTr("Display client");
            // flag=1 → close button; 6s gives the user time to read.
            W.Action.toast(qsTr("%1 connection failed: %2").arg(who).arg(reason), 6000, 1, null);
        }
    }

    W.DaemonNotRunDialog {}

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
                    onAutoExpandChanged: if (autoExpand) expanded = useEmbed

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

                            MD.RailItem {
                                width: parent.width
                                expand: m_rail.useLarge
                                checked: false
                                icon.name: MD.Token.icon.extension
                                text: "Plugins"
                                onClicked: MD.Util.showPopup('waywallen.ui/PagePopup', {
                                    source: 'waywallen.ui/PluginManagePage'
                                }, win)
                            }

                            MD.RailItem {
                                width: parent.width
                                expand: m_rail.useLarge
                                checked: false
                                icon.name: MD.Token.icon.settings
                                text: "Settings"
                                onClicked: MD.Util.showPopup('waywallen.ui/PagePopup', {
                                    source: 'waywallen.ui/SettingsPage'
                                }, win)
                            }

                            MD.RailItem {
                                visible: m_rail.useLarge
                                width: parent.width
                                expand: true
                                checked: false
                                icon.name: MD.Token.icon.info
                                text: "About"
                                onClicked: MD.Util.showPopup('waywallen.ui/PagePopup', {
                                    source: 'waywallen.ui/AboutPage'
                                }, win)
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

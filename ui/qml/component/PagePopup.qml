import QtQuick
import QtQuick.Window
import QtQuick.Templates as T

import Qcm.Material as MD
import waywallen.ui as W

MD.Popup {
    id: root
    property bool fillHeight: false
    property bool fillWidth: false
    property var props: ({})
    property var initialRequest: null
    property var pendingRequest: null
    required property string source

    readyForOpen: false
    parent: T.Overlay.overlay
    width: Math.min(400, parent.width)
    height: Math.min(implicitHeight, parent.height * 0.8)

    mdState.textColor: MD.MProp.color.on_surface
    mdState.backgroundColor: MD.MProp.color.surface
    MD.MProp.backgroundColor: MD.MProp.color.surface

    x: Math.round((parent.width - width) / 2)
    y: Math.round((parent.height - height) / 2)

    Binding on height {
        value: root.parent.height
        when: root.fillHeight
    }
    Binding on width {
        value: root.parent.width
        when: root.fillWidth
    }

    radius: MD.MProp.size.isCompact ? 0 : MD.Token.shape.corner.large
    modal: !MD.MProp.size.isCompact

    Binding {
        when: root.MD.MProp.size.isCompact
        root.fillHeight: true
        root.fillWidth: true
        root.padding: 0
        root.verticalPadding: 0
    }

    function acceptPage(request, operation) {
        const object = request.object;
        const item = m_stack.push(object, operation) as Item;
        if (item !== object)
            return null;

        const attach = item.T.StackView;
        attach.removed.connect(request, function() {
            request.release();
        });
        attach.statusChanged.connect(item, function() {
            if (!attach || !m_stack)
                return;

            if (attach.status === T.StackView.Active) {
                m_stack.lastImplicitHeight = 0;
            } else if (attach.status === T.StackView.Deactivating) {
                m_stack.lastImplicitHeight = Qt.binding(function() {
                    return item.implicitHeight;
                });
            }
        });
        m_stack.lastImplicitHeight = m_stack.implicitHeight;
        return item;
    }

    function rejectInitialRequest(request, error) {
        root.initialRequest = null;
        W.Global.toastError(error);
        request.release();
        root.rejectOpen(error);
    }

    function handleInitialRequest() {
        const request = root.initialRequest;
        if (!request)
            return;

        if (request.status === MD.PoolRequest.Ready) {
            root.initialRequest = null;
            const item = root.acceptPage(request, T.StackView.Immediate);
            if (!item) {
                root.rejectInitialRequest(request, qsTr("Failed to open page"));
                return;
            }

            Qt.callLater(function() {
                if (m_stack.currentItem === item)
                    root.readyForOpen = true;
            });
        } else if (request.status === MD.PoolRequest.Error) {
            root.rejectInitialRequest(request,
                                      request.errorString || qsTr("Failed to load page"));
        }
    }

    function handlePendingRequest() {
        const request = root.pendingRequest;
        if (!request)
            return;

        if (request.status === MD.PoolRequest.Ready) {
            root.pendingRequest = null;
            if (!root.acceptPage(request, T.StackView.PushTransition)) {
                W.Global.toastError(qsTr("Failed to open page"));
                request.release();
            }
        } else if (request.status === MD.PoolRequest.Error) {
            root.pendingRequest = null;
            W.Global.toastError(request.errorString || qsTr("Failed to load page"));
            request.release();
        } else if (request.status === MD.PoolRequest.Cancelled) {
            root.pendingRequest = null;
        }
    }

    function pushPage(source, properties) {
        if (root.pendingRequest)
            root.pendingRequest.cancel();
        root.pendingRequest = m_pool.request(source, properties, null,
                                             MD.Pool.AsynchronousIfNested);
        root.handlePendingRequest();
    }

    MD.PageContext {
        id: m_page_context
        showHeader: true
        backgroundRadius: root.radius
        radius: root.radius
        leadingAction: MD.Action {
            icon.name: MD.Token.icon.arrow_back
            onTriggered: {
                const cur = m_stack.currentItem;
                if (cur?.canBack) {
                    cur.back();
                } else if (m_stack.depth > 1) {
                    m_stack.pop();
                } else {
                    root.close();
                }
            }
        }
    }

    MD.Pool {
        id: m_pool
    }

    Connections {
        target: root.initialRequest

        function onStatusChanged() {
            root.handleInitialRequest();
        }
    }

    Connections {
        target: root.pendingRequest

        function onStatusChanged() {
            root.handlePendingRequest();
        }
    }

    Component.onCompleted: {
        root.initialRequest = m_pool.request(source, props, null, MD.Pool.Asynchronous);
        root.handleInitialRequest();
    }

    Component.onDestruction: {
        if (root.initialRequest)
            root.initialRequest.cancel();
        if (root.pendingRequest)
            root.pendingRequest.cancel();
    }

    function preparePageClose(page) {
        if (page && typeof page.prepareClose === "function")
            page.prepareClose();
    }

    onAboutToHide: root.preparePageClose(m_stack.currentItem)

    contentItem: MD.StackView {
        id: m_stack
        property real lastImplicitHeight: 0
        implicitHeight: Math.max(lastImplicitHeight, currentItem?.implicitHeight ?? 0)

        MD.MProp.page: m_page_context
        Connections {
            target: m_page_context

            function onPushItem(comp, props) {
                root.pushPage(comp, props);
            }

            function onPop() {
                m_stack.pop();
            }
        }
    }
    closePolicy: T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutside
}

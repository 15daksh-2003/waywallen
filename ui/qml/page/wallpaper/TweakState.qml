pragma ComponentBehavior: Bound
import QtCore
import QtQml

QtObject {
    id: root

    readonly property int layoutFillCell: 0
    readonly property int layoutFixed: 1
    readonly property int minimumItemSize: 112
    readonly property int maximumItemSize: 260
    readonly property int itemSizeStep: 8
    property string settingsCategory: "WallpaperView"
    property int itemSize: 162
    property real itemAspectRatio: 1
    property int layoutMode: layoutFillCell
    readonly property real itemHeight: itemSize / Math.max(itemAspectRatio, 0.1)

    readonly property Settings settings: Settings {
        category: root.settingsCategory
        property alias itemSize: root.itemSize
        property alias itemAspectRatio: root.itemAspectRatio
        property alias layoutMode: root.layoutMode
    }

    Component.onCompleted: {
        setItemSize(itemSize);
        setItemAspectRatio(itemAspectRatio);
        setLayoutMode(layoutMode);
    }

    function setItemSize(size) {
        const stepped = Math.round(Number(size) / itemSizeStep) * itemSizeStep;
        itemSize = Math.max(minimumItemSize, Math.min(maximumItemSize, stepped));
    }

    function setItemAspectRatio(ratio) {
        const next = Number(ratio);
        itemAspectRatio = next > 0 ? next : 1;
    }

    function setLayoutMode(mode) {
        layoutMode = mode === layoutFixed ? layoutFixed : layoutFillCell;
    }
}

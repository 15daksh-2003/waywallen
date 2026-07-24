pragma ComponentBehavior: Bound
import QtCore
import QtQml

QtObject {
    id: root

    property string settingsCategory: "DiscoverView"

    readonly property Settings settings: Settings {
        category: root.settingsCategory
        property string selectedSourceId: ""
        property string filtersJson: "{}"
    }

    function selectedSourceId() {
        return settings.selectedSourceId;
    }

    function setSelectedSourceId(sourceId) {
        const value = String(sourceId ?? "");
        if (settings.selectedSourceId !== value)
            settings.selectedSourceId = value;
    }

    function normalize(values) {
        const seen = {};
        const out = [];
        for (const raw of values ?? []) {
            const value = String(raw);
            if (value.length === 0 || seen[value] === true)
                continue;
            seen[value] = true;
            out.push(value);
        }
        return out;
    }

    function readFilters() {
        try {
            const state = JSON.parse(settings.filtersJson);
            if (state && typeof state === "object" && !Array.isArray(state))
                return state;
        } catch (error) {
        }
        return {};
    }

    function filterValuesFor(sourceId) {
        const values = readFilters()[sourceId];
        return Array.isArray(values) ? normalize(values) : [];
    }

    function setFilterValuesFor(sourceId, values) {
        if (sourceId.length === 0)
            return;
        const state = readFilters();
        const normalized = normalize(values);
        if (normalized.length > 0)
            state[sourceId] = normalized;
        else
            delete state[sourceId];
        const next = JSON.stringify(state);
        if (settings.filtersJson !== next)
            settings.filtersJson = next;
    }
}

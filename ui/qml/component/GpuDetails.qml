pragma Singleton
import QtQml

QtObject {
    function toolTipText(gpu: var): string {
        if (!gpu) return "";

        const lines = [];
        if (gpu.name) lines.push(gpu.name);
        if (gpu.driver) lines.push(qsTr("Driver: %1").arg(gpu.driver));

        let pci = "";
        if (gpu.pciBdf) pci = gpu.pciBdf;
        if (gpu.vendorId || gpu.deviceId) {
            const id = _hex4(gpu.vendorId) + ":" + _hex4(gpu.deviceId);
            pci = pci ? pci + " · " + id : id;
        }
        if (pci) lines.push(qsTr("PCI: %1").arg(pci));
        if (gpu.renderNode)
            lines.push(qsTr("Render node: %1").arg(gpu.renderNode));
        return lines.join("\n");
    }

    function _hex4(value: int): string {
        return Number(value).toString(16).padStart(4, "0");
    }
}

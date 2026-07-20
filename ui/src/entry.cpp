module;

#include <QCommandLineParser>
#include <QGuiApplication>
#include <QtQml/QQmlExtensionPlugin>

Q_IMPORT_QML_PLUGIN(waywallen_uiPlugin)

module waywallen.entry;

import ncrequest;
import rstd.cppstd;
import waywallen;

namespace waywallen
{
int run(int argc, char** argv) {
    auto request_init = ncrequest::global_init();
    if (request_init.is_err()) {
        auto error = rstd::cppstd::to_string(
            rstd::format("ncrequest initialization failed: {}", request_init.unwrap_err()));
        qCritical("%s", error.c_str());
        return 1;
    }

    QGuiApplication gui_app(argc, argv);
    gui_app.setDesktopFileName(APP_ID);
    gui_app.setOrganizationName("waywallen");
    gui_app.setOrganizationDomain("waywallen.org");
    gui_app.setApplicationName(APP_NAME);
    gui_app.setApplicationVersion(APP_VERSION);

    QCommandLineParser parser;
    parser.addHelpOption();
    parser.addVersionOption();
    parser.addOption(
        { "ws-port", "Override the WebSocket port (normally discovered via DBus).", "port" });
    parser.process(gui_app);

    quint16 ws_port = 0;
    if (parser.isSet("ws-port")) {
        bool ok = false;
        ws_port = parser.value("ws-port").toUShort(&ok);
        if (! ok) {
            qCritical("invalid --ws-port value: %s", qPrintable(parser.value("ws-port")));
            return 1;
        }
    }

    App app(ws_port, {});
    app.init();

    return gui_app.exec();
}
} // namespace waywallen

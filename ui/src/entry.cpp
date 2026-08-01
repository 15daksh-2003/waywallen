module;

#include <QCommandLineParser>
#include <QGuiApplication>
#include <QLibraryInfo>
#include <QLocale>
#include <QTranslator>
#include <QtQml/QQmlExtensionPlugin>

Q_IMPORT_QML_PLUGIN(waywallen_uiPlugin)

module waywallen.entry;

import ncrequest;
import rstd.cppstd;
import waywallen;

namespace
{
// Install the Qt catalog first and the app catalog second: the last installed
// translator is consulted first, so app strings win over the Qt defaults.
// Both are no-ops when the locale has no catalog, which is the English case.
void install_translators(QGuiApplication& app) {
    static QTranslator qt_translator;
    if (qt_translator.load(QLocale(),
                           QStringLiteral("qt"),
                           QStringLiteral("_"),
                           QLibraryInfo::path(QLibraryInfo::TranslationsPath))) {
        app.installTranslator(&qt_translator);
    }

    // Embedded by qt_add_translations() (see ui/CMakeLists.txt).
    static QTranslator app_translator;
    if (app_translator.load(QLocale(),
                            QStringLiteral("waywallen"),
                            QStringLiteral("_"),
                            QStringLiteral(":/i18n"))) {
        app.installTranslator(&app_translator);
    }
}
} // namespace

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

    // Before the QML engine is created, so the first frame is already localized.
    install_translators(gui_app);

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

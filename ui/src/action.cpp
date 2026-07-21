module;
#include "waywallen/action.moc.h"
#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>

module waywallen;
import :action;
import :app;

namespace waywallen
{

auto Action::instance() -> Action* {
    static Action* the = new Action(App::instance());
    return the;
}

Action* Action::create(QQmlEngine*, QJSEngine*) {
    auto a = instance();
    QJSEngine::setObjectOwnership(a, QJSEngine::CppOwnership);
    return a;
}

Action::Action(QObject* parent): QObject(parent) {}
Action::~Action() = default;

auto Action::wallpaperSelectStorage() const -> QObject* { return m_wallpaper_select_storage; }

void Action::copyToClipboard(const QString& text) { QGuiApplication::clipboard()->setText(text); }

void Action::enterWallpaperSelect(QObject* storage) {
    if (m_wallpaper_select_storage != storage) {
        m_wallpaper_select_storage = storage;
        Q_EMIT wallpaperSelectStorageChanged();
    }
    Q_EMIT wallpaperSelectEntered(storage);
}

} // namespace waywallen

#include "waywallen/action.moc.cpp"

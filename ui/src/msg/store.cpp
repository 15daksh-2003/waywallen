module;
#include "waywallen/msg/store.moc.h"

module waywallen;
import :msg.store;

namespace waywallen
{

namespace
{
auto store_instance() -> AppStore* {
    static AppStore* instance { new AppStore(App::instance()) };
    return instance;
}
} // namespace

AppStore::AppStore(QObject* parent): QObject(parent), wallpapers(), remotes() {}

AppStore::~AppStore() {}

auto AppStore::instance() -> AppStore* { return store_instance(); }

AppStore* AppStore::create(QQmlEngine*, QJSEngine*) {
    auto self = store_instance();
    QJSEngine::setObjectOwnership(self, QJSEngine::ObjectOwnership::CppOwnership);
    return self;
}

void AppStore::setRemoteAcquisitionState(const QString& sourceId, const QString& itemId,
                                         int state) {
    auto key = model::remoteKey(sourceId, itemId);
    auto row = remotes.store_query(key);
    if (! row || row->acquisitionState == state) return;
    row->acquisitionState = state;
    remotes.store_changed_callback(std::span { &key, 1 });
}

} // namespace waywallen

#include "waywallen/msg/store.moc.cpp"

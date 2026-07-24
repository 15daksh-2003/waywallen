module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/msg/store.moc"
#endif

export module waywallen:msg.store;
export import :msg.backend_msg;
export import :model.remote;
export import qextra;

namespace waywallen
{

export class AppStore : public QObject {
    Q_OBJECT
    QML_NAMED_ELEMENT(Store)
    QML_SINGLETON
public:
    AppStore(QObject* parent = nullptr);
    ~AppStore();

    static auto      instance() -> AppStore*;
    static AppStore* create(QQmlEngine*, QJSEngine*);

    using wallpaper_store = kstore::ItemTrait<model::Wallpaper>::store_type;
    using remote_store    = kstore::ItemTrait<model::RemoteRow>::store_type;

    wallpaper_store wallpapers;
    remote_store    remotes;

    void setRemoteAcquisitionState(const QString& sourceId, const QString& itemId, int state);
};

} // namespace waywallen
